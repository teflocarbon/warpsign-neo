"""Generate reference fixtures from the Python warpsign implementation.

warpsign-neo must reproduce Python warpsign's behaviour exactly: bundle IDs
registered by one have to come out identical from the other, or re-signed
apps lose their data. This script runs the Python code over a set of cases
and writes its outputs to fixtures/*.json for the Rust tests to compare
against.

Usage:
    python tools/gen_fixtures.py /path/to/warpsign
"""

import base64
import contextlib
import io
import json
import platform
import plistlib
import random
import string
import subprocess
import sys
import tempfile
from pathlib import Path

FIXTURES_DIR = Path(__file__).resolve().parent.parent / "fixtures"

if len(sys.argv) != 2:
    sys.exit(__doc__)

WARPSIGN_DIR = Path(sys.argv[1]).resolve()
sys.path.insert(0, str(WARPSIGN_DIR))

from warpsign.src.core.bundle_mapper import BundleMapping  # noqa: E402
from warpsign.src.ipa import ipa_inspector  # noqa: E402
from warpsign.src.ipa.app_patcher import (  # noqa: E402
    AppPatcher,
    PatchingOptions,
    StatusBarStyle,
    UIStyle,
)
from warpsign.src.ipa.entitlements_processor import EntitlementsProcessor  # noqa: E402
from warpsign.src.apple.developer_portal_api import DeveloperPortalAPI  # noqa: E402

TEAM_A = "ABCDE12345"
TEAM_B = "QWERT67890"
NEW_TEAM = "ZZZZZ99999"


def source_info() -> dict:
    commit = subprocess.run(
        ["git", "-C", str(WARPSIGN_DIR), "rev-parse", "HEAD"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()
    return {"warpsign_commit": commit, "python": platform.python_version()}


def write(name: str, cases: list) -> None:
    FIXTURES_DIR.mkdir(exist_ok=True)
    path = FIXTURES_DIR / f"{name}.json"
    with open(path, "w") as f:
        json.dump({"source": source_info(), "cases": cases}, f, indent=2, sort_keys=True)
        f.write("\n")
    print(f"Wrote {len(cases)} cases to {path}")


# --- Random ID generation -------------------------------------------------


def random_id_cases() -> list:
    ids = [
        "com.example.app",
        "com.example.app.widget",
        "a",
        "x.y",
        "com.some-company.My_App2",
        "io.github.verylongbundleidentifiercomponentthatkeepsgoingandgoing",
        "123.456.789",
        "com..double",
        "group.com.example.shared",
    ]
    cases = []
    for team_id in [NEW_TEAM, TEAM_A]:
        for original_id in ids:
            mapper = BundleMapping([TEAM_A], team_id, "com.example.app", randomize=True)
            cases.append(
                {
                    "team_id": team_id,
                    "original_id": original_id,
                    "random_id": mapper.gen_random_id(original_id),
                }
            )
    return cases


def python_random_cases() -> list:
    """Raw Python RNG outputs, so the Mersenne Twister port can be tested directly"""
    cases = []
    for seed in ["", "a", "com" + NEW_TEAM, "x" * 300, "ünïcödé"]:
        rng = random.Random(seed)
        values = [rng.random() for _ in range(5)]
        rng = random.Random(seed)
        choices = "".join(rng.choices(string.ascii_lowercase + string.digits, k=20))
        cases.append({"seed": seed, "random": values, "choices": choices})
    return cases


# --- Bundle mapping -------------------------------------------------------


def prepare_mapper(case: dict):
    """Mirror how SignOrchestrator sets up BundleMapping before patching.

    Returns (mapper, mapped_bundle_ids, app_groups, icloud_containers), with
    the mapper's registered identifiers set.
    """
    mapper = BundleMapping(
        original_team_ids=case["original_team_ids"],
        new_team_id=case["new_team_id"],
        original_base_id=case["main_bundle_id"],
        randomize=case["randomize"],
    )
    mapper.force_original_id = case["force_original_id"]
    mapper.profile_type = case["profile_type"]

    # _analyze_components: map each component's bundle ID and collect resources
    app_groups, icloud_containers = set(), set()
    mapped_bundle_ids = []
    for component in case["components"]:
        mapped_bundle_ids.append(mapper.map_bundle_id(component["bundle_id"]))
        groups, containers = mapper.extract_resources_from_entitlements(
            component["entitlements"]
        )
        app_groups.update(groups)
        icloud_containers.update(containers)

    # _register_app_resources: record which identifiers were registered
    def originals_of(new_id):
        return next(
            (k for k, v in mapper.mappings.items() if v.new_id == new_id), None
        )

    registered = set()
    for new_id in sorted(app_groups) + sorted(icloud_containers) + mapped_bundle_ids:
        original = originals_of(new_id)
        if original:
            registered.add(original)
        registered.add(new_id)
    for component in case["components"]:
        for team_id in mapper.original_team_ids:
            if team_id in component["bundle_id"]:
                registered.update({team_id, mapper.team_id})
    mapper.registered_identifiers = registered
    return mapper, mapped_bundle_ids, app_groups, icloud_containers


def run_mapping_case(case: dict) -> dict:
    """Mirror how SignOrchestrator drives BundleMapping during a sign"""
    mapper, mapped_bundle_ids, app_groups, icloud_containers = prepare_mapper(case)
    registered = mapper.registered_identifiers

    # _sign_components: map entitlements against the Info.plist bundle ID
    mapped_entitlements = [
        mapper.map_entitlements(c["entitlements"], override_bundle_id=bundle_id)
        if c["entitlements"]
        else {}
        for c, bundle_id in zip(case["components"], mapped_bundle_ids)
    ]

    return {
        "mapped_bundle_ids": mapped_bundle_ids,
        "app_groups": sorted(app_groups),
        "icloud_containers": sorted(icloud_containers),
        "registered_identifiers": sorted(registered),
        "mapped_entitlements": mapped_entitlements,
        "mappings": {
            k: {"new_id": v.new_id, "id_type": v.id_type.name}
            for k, v in mapper.mappings.items()
        },
        "binary_patches": mapper.get_binary_patches(),
    }


def mapping_case(name, components, **overrides) -> dict:
    case = {
        "name": name,
        "original_team_ids": [TEAM_A],
        "new_team_id": NEW_TEAM,
        "main_bundle_id": "com.example.app",
        "randomize": True,
        "force_original_id": False,
        "profile_type": "development",
        "components": components,
    }
    case.update(overrides)
    return case


def component(bundle_id, **entitlements) -> dict:
    return {"bundle_id": bundle_id, "entitlements": entitlements}


FULL_ENTITLEMENTS = {
    "application-identifier": f"{TEAM_A}.com.example.app",
    "com.apple.developer.team-identifier": TEAM_A,
    "aps-environment": "production",
    "get-task-allow": False,
    "keychain-access-groups": [
        f"{TEAM_A}.com.example.app",
        f"{TEAM_A}.shared",
        "com.apple.token",
    ],
    "com.apple.security.application-groups": [
        "group.com.example.app",
        f"group.{TEAM_A}.shared",
    ],
    "com.apple.developer.icloud-container-identifiers": [
        "iCloud.com.example.app",
        f"{TEAM_A}.com.example.legacy",
    ],
    "com.apple.developer.ubiquity-container-identifiers": [
        f"{TEAM_A}.com.example.legacy",
    ],
    "com.apple.developer.icloud-services": ["CloudKit", "CloudDocuments"],
    "com.apple.developer.ubiquity-kvstore-identifier": f"{TEAM_A}.com.example.app",
    "com.apple.developer.associated-domains": ["applinks:example.com"],
}

WIDGET_ENTITLEMENTS = {
    "application-identifier": f"{TEAM_A}.com.example.app.widget",
    "com.apple.developer.team-identifier": TEAM_A,
    "com.apple.security.application-groups": ["group.com.example.app"],
    "keychain-access-groups": [f"{TEAM_A}.com.example.app.widget"],
}


def full_app_components() -> list:
    return [
        component("com.example.app", **FULL_ENTITLEMENTS),
        component("com.example.app.widget", **WIDGET_ENTITLEMENTS),
        component(
            "com.example.app.share",
            **{
                "application-identifier": f"{TEAM_A}.com.example.app.share",
                "com.apple.security.application-groups": ["group.com.example.app"],
            },
        ),
    ]


def mapping_cases() -> list:
    full_app = full_app_components()
    cases = [
        mapping_case("full_app_development", full_app),
        mapping_case("full_app_adhoc", full_app, profile_type="adhoc"),
        mapping_case("full_app_no_encode_ids", full_app, randomize=False),
        mapping_case("full_app_force_original_id", full_app, force_original_id=True),
        mapping_case(
            "multiple_team_ids",
            [
                component(
                    "com.example.app",
                    **{
                        "application-identifier": f"{TEAM_A}.com.example.app",
                        "keychain-access-groups": [
                            f"{TEAM_A}.com.example.app",
                            f"{TEAM_B}.com.example.sso",
                        ],
                    },
                ),
            ],
            original_team_ids=[TEAM_A, TEAM_B],
        ),
        # Wallet pass type IDs carry a team prefix: TEAM.* (Twitter's) or
        # TEAM.pass.example; other teams' stay as they are
        mapping_case(
            "wallet_pass_type_ids",
            [
                component(
                    "com.example.app",
                    **{
                        "application-identifier": f"{TEAM_A}.com.example.app",
                        "com.apple.developer.pass-type-identifiers": [
                            f"{TEAM_A}.*",
                            f"{TEAM_B}.pass.com.example.card",
                            "UNKNOWN123.pass.other",
                        ],
                    },
                ),
            ],
            original_team_ids=[TEAM_A, TEAM_B],
        ),
        mapping_case(
            "kvstore_short_form",
            [
                component(
                    "com.example.app",
                    **{
                        "application-identifier": f"{TEAM_A}.com.example.app",
                        "com.apple.developer.ubiquity-kvstore-identifier": f"{TEAM_A}.x",
                    },
                ),
            ],
        ),
        mapping_case(
            "icloud_in_bundle_and_group_ids",
            [
                component(
                    "com.example.app",
                    **{
                        "application-identifier": f"{TEAM_A}.com.example.app",
                        "com.apple.security.application-groups": [
                            "group.com.example.iCloudShare"
                        ],
                    },
                ),
                component(
                    "com.example.app.iCloudSync",
                    **{
                        "application-identifier": f"{TEAM_A}.com.example.app.iCloudSync",
                        "com.apple.security.application-groups": [
                            "group.com.example.iCloudShare"
                        ],
                    },
                ),
            ],
        ),
        mapping_case(
            "extension_outside_main_prefix",
            [
                component("com.example.app"),
                component(
                    "com.othervendor.helper",
                    **{"application-identifier": f"{TEAM_A}.com.othervendor.helper"},
                ),
            ],
        ),
        mapping_case(
            "no_entitlements",
            [component("com.example.app"), component("com.example.app.widget")],
        ),
    ]
    for case in cases:
        case["expected"] = run_mapping_case(case)
    return cases


# --- Entitlements processor -----------------------------------------------


def capability(cap_id, name, keys=(), option_keys=(), sdks=("iOS",), dists=("Development", "Ad hoc")):
    return {
        "id": cap_id,
        "attributes": {
            "name": name,
            "description": f"{name} capability",
            "supportedSDKs": [{"displayValue": sdk} for sdk in sdks],
            "distributionTypes": [{"displayValue": d} for d in dists],
            "entitlements": [{"profileKey": k} for k in keys],
            "settings": [
                {"options": [{"entitlements": [{"profileKey": k} for k in option_keys]}]}
            ]
            if option_keys
            else [],
        },
    }


# Synthetic response in the shape of /services-account/v1/capabilities
CAPABILITIES_RESPONSE = {
    "data": [
        capability("PUSH_NOTIFICATIONS", "Push Notifications", ["aps-environment"]),
        capability("APP_GROUPS", "App Groups", ["com.apple.security.application-groups"]),
        capability(
            "ICLOUD",
            "iCloud",
            [
                "com.apple.developer.icloud-container-identifiers",
                "com.apple.developer.ubiquity-container-identifiers",
            ],
            option_keys=[
                "com.apple.developer.icloud-services",
                "com.apple.developer.ubiquity-kvstore-identifier",
            ],
        ),
        capability(
            "ASSOCIATED_DOMAINS",
            "Associated Domains",
            ["com.apple.developer.associated-domains"],
        ),
        capability(
            "IN_APP_PAYMENTS", "Apple Pay", ["com.apple.developer.in-app-payments"]
        ),
        # Only supported for development profiles
        capability(
            "DEV_ONLY", "Development Only", ["com.example.dev-only"], dists=("Development",)
        ),
        # macOS only, so never enabled for iOS
        capability(
            "MAC_ONLY", "Mac Only", ["com.example.mac-only"], sdks=("macOS",)
        ),
        # Same key as an earlier capability: the later one wins
        capability(
            "ASSOCIATED_DOMAINS_V2",
            "Associated Domains v2",
            ["com.apple.developer.associated-domains"],
        ),
        # Capability without any profile keys is ignored
        capability("NO_KEYS", "No Keys"),
    ]
}


def entitlements_processor_cases() -> list:
    entitlement_sets = {
        "full_app": dict(FULL_ENTITLEMENTS, **{"com.apple.developer.in-app-payments": ["merchant.x"]}),
        "platform_specific": {
            "application-identifier": f"{TEAM_A}.com.example.app",
            "com.example.dev-only": True,
            "com.example.mac-only": True,
            "com.example.unknown": True,
        },
        "empty": {},
    }
    cases = []
    for profile_type in ["development", "adhoc"]:
        for name, entitlements in entitlement_sets.items():
            processor = EntitlementsProcessor(CAPABILITIES_RESPONSE, profile_type)
            enable, remove = processor.process_entitlements(entitlements)
            cases.append(
                {
                    "name": f"{name}_{profile_type}",
                    "profile_type": profile_type,
                    "entitlements": entitlements,
                    "expected": {
                        "capabilities_to_enable": sorted(enable),
                        "entitlements_to_remove": sorted(remove),
                    },
                }
            )
    return cases


def _profile_keys(value) -> set:
    """Every profileKey anywhere in a capabilities response"""
    keys = set()
    if isinstance(value, dict):
        if isinstance(value.get("profileKey"), str):
            keys.add(value["profileKey"])
        for item in value.values():
            keys |= _profile_keys(item)
    elif isinstance(value, list):
        for item in value:
            keys |= _profile_keys(item)
    return keys


def apple_entitlements_processor_cases() -> list:
    """The processor against a real capabilities response from Apple
    (fixtures/capabilities_apple.json, saved with `warpsign capabilities
    --json`, October 2026: 135 capabilities)"""
    with open(FIXTURES_DIR / "capabilities_apple.json") as f:
        response = json.load(f)
    every_key = {key: True for key in sorted(_profile_keys(response))}
    entitlement_sets = {
        # Every key Apple lists, plus the core identifiers and unknown keys
        "every_key": dict(
            every_key,
            **{
                "application-identifier": f"{TEAM_A}.com.example.app",
                "com.apple.developer.team-identifier": TEAM_A,
                "keychain-access-groups": [f"{TEAM_A}.com.example.app"],
                "get-task-allow": True,
                "com.example.unknown": True,
            },
        ),
        "typical_app": dict(FULL_ENTITLEMENTS),
        # Offered by both a development and a distribution capability
        "family_controls": {"com.apple.developer.family-controls": True},
        # Offered by two capabilities of the same kind
        "storekit_external_purchase": {
            "com.apple.developer.storekit.custom-purchase-link.allowed-regions": ["US"],
        },
        # Enhanced Security's many keys
        "hardened_process": {
            "com.apple.security.hardened-process": True,
            "com.apple.security.hardened-process.hardened-heap": True,
            "com.apple.security.hardened-process.enhanced-security-version": 1,
        },
    }
    cases = []
    for profile_type in ["development", "adhoc"]:
        for name, entitlements in entitlement_sets.items():
            processor = EntitlementsProcessor(response, profile_type)
            enable, remove = processor.process_entitlements(entitlements)
            cases.append(
                {
                    "name": f"{name}_{profile_type}",
                    "profile_type": profile_type,
                    "entitlements": entitlements,
                    "expected": {
                        "capabilities_to_enable": sorted(enable),
                        "entitlements_to_remove": sorted(remove),
                    },
                }
            )
    return cases


# --- Apple SRP login ------------------------------------------------------


def srp_cases() -> list:
    """Client values for Apple's SRP sign-in, as computed by the srp library
    Python warpsign uses, with fixed randomness. A server-side Verifier checks
    each case really authenticates."""
    import hashlib

    import srp
    import srp._pysrp as pysrp

    # Settings are per backend: srp.User may be the OpenSSL one (_ctsrp)
    for module in (srp, pysrp):
        module.rfc5054_enable()
        module.no_username_in_x()

    def derive(password: str, salt: bytes, iterations: int, protocol: str) -> bytes:
        password_hash = hashlib.sha256(password.encode()).digest()
        if protocol == "s2k_fo":
            password_hash = password_hash.hex().encode()
        return hashlib.pbkdf2_hmac("sha256", password_hash, salt, iterations, 32)

    cases = []
    for i, (email, password, protocol, iterations) in enumerate(
        [
            ("someone@example.com", "correct horse battery staple", "s2k", 20309),
            ("Someone.Else@Example.com", "pässwörd with ünïcode", "s2k_fo", 1000),
            ("short@x.io", "p", "s2k", 1),
        ]
    ):
        rng = random.Random(f"srp-{i}")
        salt = bytes(rng.getrandbits(8) for _ in range(16))
        a = bytes(rng.getrandbits(8) for _ in range(32))  # _ctsrp (used by warpsign) wants 32
        b = bytes(rng.getrandbits(8) for _ in range(256))
        derived = derive(password, salt, iterations, protocol)

        # Server side: verifier for the derived password, with this salt
        x = pysrp.gen_x(hashlib.sha256, salt, email, derived)
        n, g = pysrp.get_ng(pysrp.NG_2048, None, None)
        verification_key = pysrp.long_to_bytes(pow(g, x, n))

        user = srp.User(email, derived, hash_alg=srp.SHA256, ng_type=srp.NG_2048, bytes_a=a)
        _, big_a = user.start_authentication()
        verifier = pysrp.Verifier(
            email, salt, verification_key, big_a,
            hash_alg=pysrp.SHA256, ng_type=pysrp.NG_2048, bytes_b=b,
        )
        salt_out, big_b = verifier.get_challenge()
        m1 = user.process_challenge(salt_out, big_b)
        h_amk = verifier.verify_session(m1)
        assert verifier.authenticated() and h_amk is not None
        user.verify_session(h_amk)
        assert user.authenticated()

        cases.append(
            {
                "email": email,
                "password": password,
                "protocol": protocol,
                "iterations": iterations,
                "salt": salt.hex(),
                "a": a.hex(),
                "B": big_b.hex(),
                "expected": {
                    "derived_password": derived.hex(),
                    "A": big_a.hex(),
                    "M1": m1.hex(),
                    "M2": user.H_AMK.hex(),
                },
            }
        )
    return cases


# --- Info.plist patching --------------------------------------------------

# Info.plists for full_app_components(), main app first (the order warpsign
# patches them in). IDs appear in places only the byte-level patching reaches.
FULL_APP_INFO_PLISTS = [
    {
        "path": "",
        "info_plist": {
            "CFBundleIdentifier": "com.example.app",
            "CFBundleExecutable": "App",
            "CFBundleName": "Example",
            "CFBundleDisplayName": "Example",
            "AppIdentifierPrefix": f"{TEAM_A}.",
            "MinimumOSVersion": "15.0",
            "UIDeviceFamily": [1],
            "UISupportedDevices": ["iPhone14,2"],
            "UIStatusBarStyle": "UIStatusBarStyleDefault",
            "UIViewControllerBasedStatusBarAppearance": True,
            "CFBundleURLTypes": [
                {"CFBundleURLSchemes": ["com.example.app", "examplescheme"]}
            ],
            "NSUserActivityTypes": ["com.example.app.activity"],
            "SharedContainerGroup": "group.com.example.app",
            "com.apple.security.application-groups": ["group.com.example.app"],
            "keychain-access-groups": f"{TEAM_A}.com.example.app",
            "com.apple.developer.icloud-container-identifiers": ["iCloud.com.example.app"],
            "com.apple.developer.associated-domains": ["applinks:example.com"],
            # Remapped when they start with the app's bundle ID; others kept
            "BGTaskSchedulerPermittedIdentifiers": [
                "com.example.app.refresh",
                "com.example.app.userTask.*",
                "com.example.application.task",
                "com.vendor.sdk.sync",
            ],
        },
    },
    {
        "path": "PlugIns/Widget.appex",
        "info_plist": {
            "CFBundleIdentifier": "com.example.app.widget",
            "CFBundleExecutable": "Widget",
            "CFBundleName": "Widget",
            "MinimumOSVersion": "15.0",
            "NSExtension": {"NSExtensionPointIdentifier": "com.apple.widgetkit-extension"},
            "AppGroup": "group.com.example.app",
        },
    },
    {
        "path": "PlugIns/Share.appex",
        "info_plist": {
            "CFBundleIdentifier": "com.example.app.share",
            "CFBundleExecutable": "Share",
            "NSExtension": {
                "NSExtensionPointIdentifier": "com.apple.share-services",
                "NSExtensionAttributes": {"HostBundle": "com.example.app"},
            },
        },
    },
]

ALL_MAIN_APP_PATCHES = {
    "bundle_name": "My App",
    "patch_all_devices": True,
    "patch_file_sharing": True,
    "patch_promotion": True,
    "patch_fullscreen": True,
    "patch_orientation": True,
    "patch_game_mode": True,
    "patch_status_bar": "hidden",
    "patch_user_interface_style": "dark",
    "remove_url_schemes": True,
}


def info_plist_cases() -> list:
    option_sets = [
        ("defaults", {}, {}),
        ("all_main_app_patches", ALL_MAIN_APP_PATCHES, {}),
        ("status_bar_light_ui_light", {"patch_status_bar": "light", "patch_user_interface_style": "light"}, {}),
        ("status_bar_dark", {"patch_status_bar": "dark"}, {}),
        ("force_original_id", {"force_original_id": True}, {"force_original_id": True}),
        ("no_patch_ids", {"patch_ids": False}, {}),
        ("no_encode_ids", {}, {"randomize": False}),
        ("icon", {"icon_png": test_icon_png()}, {}),
    ]
    cases = []
    for name, options, mapping_overrides in option_sets:
        case = mapping_case(name, full_app_components(), **mapping_overrides)
        mapper, _, _, _ = prepare_mapper(case)
        case["registered_identifiers"] = sorted(mapper.registered_identifiers)
        case["options"] = options

        python_options = dict(options)
        if "patch_status_bar" in python_options:
            python_options["patch_status_bar"] = StatusBarStyle(python_options["patch_status_bar"])
        if "patch_user_interface_style" in python_options:
            python_options["patch_user_interface_style"] = UIStyle(
                python_options["patch_user_interface_style"]
            )

        case["info_plists"] = []
        with tempfile.TemporaryDirectory() as tmp:
            app = Path(tmp) / "App.app"
            app.mkdir()
            if "icon_png" in python_options:
                icon = Path(tmp) / "icon.png"
                icon.write_bytes(base64.b64decode(python_options.pop("icon_png")))
                python_options["icon_path"] = icon
            with contextlib.redirect_stdout(io.StringIO()):
                patcher = AppPatcher(app, PatchingOptions(**python_options), mapper)
                for entry in FULL_APP_INFO_PLISTS:
                    path = app / entry["path"] / "Info.plist"
                    path.parent.mkdir(parents=True, exist_ok=True)
                    with open(path, "wb") as f:
                        plistlib.dump(entry["info_plist"], f)
                    patcher.patch_info_plist(
                        path, bundle_mapper=mapper, is_main_app=entry["path"] == ""
                    )
                    with open(path, "rb") as f:
                        patched = plistlib.load(f)
                    case["info_plists"].append({**entry, "expected": patched})

                if "icon_path" in python_options:
                    from PIL import Image

                    case["icon_files"] = sorted(
                        [normalise_icon_name(f.name), list(Image.open(f).size)]
                        for f in app.iterdir()
                        if f.name.startswith("cyan_")
                    )
                    for entry in case["info_plists"]:
                        entry["expected"] = json.loads(
                            normalise_icon_name(json.dumps(entry["expected"]))
                        )

                # Then a binary, with the patcher state the plists left
                # (as SignOrchestrator does): every ID it knows, embedded
                binary = app / "Binary"
                blob = binary_blob(mapper)
                binary.write_bytes(blob)
                patcher.patch_app_binary(binary, mapper)
                case["binary"] = {
                    "input": base64.b64encode(blob).decode(),
                    "expected": base64.b64encode(binary.read_bytes()).decode(),
                }
        cases.append(case)
    return cases


def test_icon_png() -> str:
    """A small square PNG for the icon case"""
    from PIL import Image

    image = Image.new("RGB", (64, 64), (90, 60, 200))
    out = io.BytesIO()
    image.save(out, "PNG")
    return base64.b64encode(out.getvalue()).decode()


def normalise_icon_name(text: str) -> str:
    """IconHandler names icons cyan_<7 random hex>a: pin the random part"""
    import re

    return re.sub(r"cyan_[0-9a-f]{7}a", "cyan_0000000a", text)


def binary_blob(mapper) -> bytes:
    """Bytes containing every identifier a mapper knows, in various
    surroundings, like strings in a Mach-O"""
    ids = sorted(set(mapper.mappings) | set(mapper.original_team_ids))
    parts = [b"\xcf\xfa\xed\xfe\x0c\x00\x00\x01"]
    for identifier in ids:
        encoded = identifier.encode()
        parts += [encoded + b"\x00", b"prefix." + encoded + b".suffix\x00", encoded + encoded]
    parts.append(f"{mapper.original_team_ids[0]}.com.example.app\x00".encode())
    return b"".join(parts)


# --- App inspection -------------------------------------------------------


def bundle_files(path, executable, bundle_id=None, **extra_info):
    """Info.plist plus executable for a bundle at `path` (relative to the app)"""
    info = {"CFBundleExecutable": executable, **extra_info}
    if bundle_id:
        info["CFBundleIdentifier"] = bundle_id
    prefix = f"{path}/" if path else ""
    return [
        {"path": f"{prefix}Info.plist", "info_plist": info},
        {"path": f"{prefix}{executable}"},
    ]


def info_only(path, info):
    return {"path": f"{path}/Info.plist", "info_plist": info}


def app_layouts() -> list:
    """App layouts as lists of files, so Python and Rust build identical trees.

    A file is either an Info.plist ({"path", "info_plist"}) or an opaque stub
    ({"path"}). Inspection reads entitlements from binaries; here they are
    stubbed, keyed by executable path.
    """
    typical = [
        *bundle_files("", "App", "com.example.app", AppIdentifierPrefix=f"{TEAM_A}."),
        *bundle_files("Frameworks/Alpha.framework", "Alpha"),
        *bundle_files("Frameworks/Renamed.framework", "RenamedBinary"),
        {"path": "Frameworks/Legacy.framework/Legacy"},
        {"path": "Frameworks/libswiftCore.dylib"},
        info_only("Frameworks/Missing.framework", {"CFBundleExecutable": "Missing"}),
        *bundle_files("PlugIns/Widget.appex", "Widget", "com.example.app.widget"),
        *bundle_files("PlugIns/Widget.appex/Frameworks/Inner.framework", "Inner"),
        {"path": "PlugIns/Widget.appex/Frameworks/libInner.dylib"},
        *bundle_files("PlugIns/Share Extension.appex", "Share Extension", "com.example.app.share"),
        *bundle_files("PlugIns/Resources.bundle", "ResourcesBinary"),
        {"path": "PlugIns/NoInfo.appex/NoInfo"},
        info_only("PlugIns/NoExecutable.appex", {"CFBundleIdentifier": "com.example.app.noexe"}),
        info_only(
            "PlugIns/MissingBinary.appex",
            {"CFBundleIdentifier": "com.example.app.gone", "CFBundleExecutable": "Gone"},
        ),
        *bundle_files("Extensions/Intents.appex", "Intents", "com.example.app.intents"),
        {"path": "Settings.bundle/Root.plist"},
        {"path": "Assets.car"},
    ]
    watch = [
        *bundle_files("", "App", "com.example.app"),
        *bundle_files("Watch/WatchApp.app", "WatchApp", "com.example.app.watchkitapp"),
        *bundle_files(
            "Watch/WatchApp.app/PlugIns/WatchExt.appex",
            "WatchExt",
            "com.example.app.watchkitapp.ext",
        ),
    ]
    return [
        {
            "name": "typical_app",
            "files": typical,
            "entitlements": {
                "App": {
                    "application-identifier": f"{TEAM_A}.com.example.app",
                    "com.apple.developer.team-identifier": TEAM_A,
                    "keychain-access-groups": [f"{TEAM_A}.com.example.app", "com.apple.token"],
                },
                "PlugIns/Widget.appex/Widget": {
                    "application-identifier": f"{TEAM_A}.com.example.app.widget",
                },
                "Frameworks/Alpha.framework/Alpha": {"com.example.framework-entitlement": True},
            },
        },
        # Watch apps aren't discovered (only their extensions are): a known gap
        {"name": "watch_app", "files": watch, "entitlements": {}},
        {
            "name": "multiple_team_ids",
            "files": [
                *bundle_files("", "App", "com.example.app", AppIdentifierPrefix="SHORT."),
                *bundle_files("PlugIns/Sso.appex", "Sso", "com.example.app.sso"),
            ],
            "entitlements": {
                "App": {
                    "application-identifier": f"{TEAM_A}.com.example.app",
                    "com.apple.developer.team-identifier": TEAM_A,
                    "keychain-access-groups": [
                        f"{TEAM_B}.com.example.shared",
                        "com.apple.token",
                        "noprefixgroup",
                        "ab-de12345.not-alphanumeric",
                    ],
                },
                "PlugIns/Sso.appex/Sso": {
                    "application-identifier": "CCCCC33333.com.example.app.sso",
                },
            },
        },
    ]


def build_layout(root: Path, layout: dict) -> Path:
    app = root / "App.app"
    for file in layout["files"]:
        path = app / file["path"]
        path.parent.mkdir(parents=True, exist_ok=True)
        if "info_plist" in file:
            with open(path, "wb") as f:
                plistlib.dump(file["info_plist"], f)
        else:
            path.write_bytes(b"stub")
    return app


def inspection_cases() -> list:
    cases = []
    for layout in app_layouts():
        with tempfile.TemporaryDirectory() as tmp:
            app = build_layout(Path(tmp), layout)

            def fake_dump(executable: str):
                relative = str(Path(executable).relative_to(app))
                return layout["entitlements"].get(relative, {})

            original_dump = ipa_inspector.codesign_dump_entitlements
            ipa_inspector.codesign_dump_entitlements = fake_dump
            try:
                with contextlib.redirect_stdout(io.StringIO()):
                    with ipa_inspector.IPAInspector(app) as inspector:
                        components = inspector.get_components()
                        team_ids = inspector.get_team_ids()
            finally:
                ipa_inspector.codesign_dump_entitlements = original_dump

        # Python's order within a depth follows the filesystem; pin a stable one
        main = [c for c in components if c.path == Path(".")]
        rest = sorted(
            (c for c in components if c.path != Path(".")),
            key=lambda c: (-len(c.path.parts), str(c.path)),
        )
        layout["expected"] = {
            "components": [
                {
                    "path": "" if c.path == Path(".") else str(c.path),
                    "bundle_id": c.bundle_id,
                    "executable": str(c.executable),
                    "is_primary": c.is_primary,
                    "entitlements_bundle_id": c.entitlements_bundle_id,
                    "entitlements": c.entitlements,
                }
                for c in main + rest
            ],
            "team_ids": team_ids,
        }
        cases.append(layout)
    return cases


# --- Developer Portal requests ---------------------------------------------

# Headers the HTTP stack owns, which neo leaves to the platform transport
TRANSPORT_HEADERS = {"user-agent", "accept-encoding", "connection", "host", "content-length", "cookie"}


class RecordingAdapter:
    """A requests transport adapter that records each request and answers
    with the next canned response"""

    def __init__(self, responses):
        self.responses = list(responses)
        self.sent = []

    def send(self, request, **kwargs):
        import requests
        from requests.structures import CaseInsensitiveDict

        self.sent.append(request)
        canned = self.responses.pop(0)
        response = requests.Response()
        response.status_code = canned["status"]
        response.headers = CaseInsensitiveDict(canned.get("headers", {}))
        if "json" in canned:
            response._content = json.dumps(canned["json"]).encode()
        else:
            response._content = canned.get("text", "").encode()
        response.url = request.url
        response.request = request
        response.encoding = "utf-8"
        return response

    def close(self):
        pass


def recorded_request(request) -> dict:
    headers = {k: v for k, v in request.headers.items() if k.lower() not in TRANSPORT_HEADERS}
    body = request.body or b""
    if isinstance(body, str):
        body = body.encode()
    recorded = {"method": request.method, "url": request.url, "headers": headers}
    content_type = headers.get("Content-Type", "")
    if body and "json" in content_type:
        recorded["json"] = json.loads(body)
    else:
        recorded["body"] = body.decode()
    return recorded


def jsonable(value):
    import dataclasses
    import base64

    if dataclasses.is_dataclass(value):
        return dataclasses.asdict(value)
    if isinstance(value, list):
        return [jsonable(v) for v in value]
    if isinstance(value, bytes):
        return {"base64": base64.b64encode(value).decode()}
    return value


def run_portal_case(name, call, args, responses) -> dict:
    import requests

    adapter = RecordingAdapter(responses)
    session = requests.Session()
    session.mount("https://", adapter)

    class Auth:
        pass

    auth = Auth()
    auth.session = session
    auth.csrf = "csrf-token"
    auth.csrf_ts = "1700000000000"
    api = DeveloperPortalAPI(auth)
    with contextlib.redirect_stdout(io.StringIO()):
        result = getattr(api, call)(**args)
    assert not adapter.responses, f"{name}: unused responses"
    return {
        "name": name,
        "call": call,
        "args": args,
        "responses": responses,
        "requests": [recorded_request(r) for r in adapter.sent],
        "result": jsonable(result),
    }


def resource(kind, rid, **attributes):
    return {"type": kind, "id": rid, "attributes": attributes}


# /v1/capabilities for the portal cases: the processor's synthetic response
# plus a required capability and one with settings warpsign doesn't know
def with_distribution_names(capabilities):
    """Python's portal client reads distributionTypes[].name, the processor
    reads displayValue: the real response must have both"""
    import copy

    capabilities = copy.deepcopy(capabilities)
    for item in capabilities:
        for dist in item["attributes"].get("distributionTypes", []):
            dist["name"] = dist["displayValue"].upper().replace(" ", "_")
    return capabilities


PORTAL_CAPABILITIES = {
    "data": with_distribution_names(CAPABILITIES_RESPONSE["data"])
    + [
        {
            "id": "IN_APP_PURCHASE",
            "attributes": {"name": "In-App Purchase", "optional": False, "entitlements": [], "settings": []},
        },
        {
            "id": "NEW_THING",
            "attributes": {
                "name": "Something New",
                "entitlements": [{"profileKey": "com.apple.developer.new-thing"}],
                "settings": [{"key": "NEW_THING_MODE", "options": [{"key": "A"}, {"key": "B"}]}],
            },
        },
    ]
}


def portal_cases() -> list:
    team = NEW_TEAM
    taken = {"resultCode": 0, "userString": "An App Group with Identifier 'x' is not available. Please enter a different string."}
    return [
        run_portal_case(
            "teams",
            "list_teams",
            {},
            [{"status": 200, "json": {"resultCode": 0, "teams": [
                {"teamId": team, "name": "Test Team", "status": "active", "entityType": "INDIVIDUAL", "userRoles": ["TEAM_ADMIN"]},
            ]}}],
        ),
        run_portal_case(
            "certificates",
            "list_certificates",
            {"team_id": team},
            [{"status": 200, "json": {"data": [
                resource("certificates", "CERT1", serialNumber="5A1B2C3D4E5F6071", ownerId="OWNER1", certificateType="DEVELOPMENT", name="Apple Development: Test User", displayName="Test User"),
                resource("certificates", "CERT2", serialNumber="0FEDCBA987654321", ownerId="OWNER1", certificateType="DISTRIBUTION", name="Apple Distribution: Test User"),
            ]}}],
        ),
        run_portal_case(
            "bundle_ids",
            "list_bundle_ids",
            {"team_id": team},
            [{"status": 200, "json": {"data": [
                resource("bundleIds", "BUNDLE1", identifier="com.example.app", name="WS App app"),
            ]}}],
        ),
        run_portal_case(
            "app_groups",
            "list_app_group_ids",
            {"team_id": team},
            [{"status": 200, "json": {"resultCode": 0, "applicationGroupList": [
                {"applicationGroup": "GROUP1", "identifier": "group.com.example.app", "name": "WS App Group"},
            ]}}],
        ),
        run_portal_case(
            "cloud_containers",
            "list_icloud_container_ids",
            {"team_id": team},
            [{"status": 200, "json": {"data": [
                resource("cloudContainers", "CLOUD1", identifier="iCloud.com.example.app", name="WS iCloud"),
            ]}}],
        ),
        run_portal_case(
            "devices",
            "list_devices",
            {"team_id": team},
            [{"status": 200, "json": {"data": [
                resource("devices", "DEV1", name="iPhone", udid="00008110-0001", status="ENABLED", deviceClass="IPHONE", platform="IOS", model="iPhone 15"),
                resource("devices", "DEV2", name="Mac", udid="MAC-1", status="ENABLED", deviceClass="MAC", platform="MAC_OS", model=None),
                resource("devices", "DEV3", name="iPad", udid="00008110-0003", status="ENABLED", deviceClass="IPAD", platform="IOS", model=None),
            ]}}],
        ),
        run_portal_case(
            "profiles",
            "list_profiles",
            {"team_id": team},
            [{"status": 200, "json": {"data": [
                resource("profiles", "PROF1", name="TS com.example.app Development", profileState="ACTIVE", platform="IOS", profileTypeLabel="iOS App Development"),
            ]}}],
        ),
        run_portal_case(
            "capabilities",
            "fetch_available_user_entitlements",
            {"team_id": team, "return_raw": True},
            [{"status": 200, "json": PORTAL_CAPABILITIES}],
        ),
        run_portal_case(
            "set_capabilities",
            "set_entitlements_for_bundle_id",
            {
                "team_id": team,
                "bundle_id_resource_id": "BUNDLE1",
                "bundle_identifier": "com.example.app",
                "capabilities_to_enable": ["PUSH_NOTIFICATIONS", "APP_GROUPS", "ICLOUD", "ASSOCIATED_DOMAINS", "NEW_THING"],
                "group_ids": {"APP_GROUPS": ["GROUP1", "GROUP2"], "ICLOUD": ["CLOUD1"]},
            },
            [{"status": 200, "json": PORTAL_CAPABILITIES}, {"status": 200, "json": {"data": {}}}],
        ),
        run_portal_case(
            "register_bundle_id",
            "register_bundle_id",
            {"team_id": team, "identifier": "com.example.app", "name": "WS App app"},
            [{"status": 201, "json": {"data": resource("bundleIds", "BUNDLE1", identifier="com.example.app", name="WS App app")}}],
        ),
        run_portal_case(
            "register_existing_bundle_id",
            "register_bundle_id",
            {"team_id": team, "identifier": "com.example.app", "name": "WS App app"},
            [
                {"status": 409, "json": {"errors": [{"status": "409", "resultCode": 9400, "detail": "An App ID with Identifier 'com.example.app' is not available."}]}},
                {"status": 200, "json": {"data": [
                    resource("bundleIds", "OTHER", identifier="com.example.app.widget", name="WS Widget"),
                    resource("bundleIds", "BUNDLE1", identifier="com.example.app", name="WS App app"),
                ]}},
            ],
        ),
        run_portal_case(
            "register_app_group",
            "register_app_group",
            {"team_id": team, "identifier": "group.com.example.app", "name": "WS App Group group com example app"},
            [{"status": 200, "json": {"resultCode": 0, "applicationGroup": {"applicationGroup": "GROUP1", "identifier": "group.com.example.app", "name": "WS App Group group com example app"}}}],
        ),
        run_portal_case(
            "register_taken_app_group",
            "register_app_group",
            {"team_id": team, "identifier": "group.com.example.app", "name": "WS App Group group com example app"},
            [
                {"status": 200, "json": taken},
                {"status": 200, "json": {"resultCode": 0, "applicationGroupList": [
                    {"applicationGroup": "GROUP1", "identifier": "group.com.example.app", "name": "WS App Group"},
                ]}},
            ],
        ),
        run_portal_case(
            "register_cloud_container",
            "register_icloud_container",
            {"team_id": team, "identifier": "iCloud.com.example.app", "name": "WS iCloud Container iCloud com example app"},
            [{"status": 200, "json": {"resultCode": 0, "cloudContainer": {"cloudContainer": "CLOUD1", "identifier": "iCloud.com.example.app", "name": "WS iCloud Container iCloud com example app"}}}],
        ),
        run_portal_case(
            "register_taken_cloud_container",
            "register_icloud_container",
            {"team_id": team, "identifier": "iCloud.com.example.app", "name": "WS iCloud Container iCloud com example app"},
            [
                {"status": 200, "json": taken},
                {"status": 200, "json": {"data": [
                    resource("cloudContainers", "CLOUD1", identifier="iCloud.com.example.app", name="WS iCloud"),
                ]}},
            ],
        ),
        run_portal_case(
            "create_profile",
            "create_or_regen_provisioning_profile",
            {
                "team_id": team,
                "profile_id": "",
                "app_id_id": "BUNDLE1",
                "profile_name": "TS com.example.app Development",
                "certificate_ids": ["CERT1"],
                "device_ids": ["DEV1", "DEV3"],
                "distribution_type": "development",
            },
            [
                {"status": 200, "json": {"resultCode": 0, "provisioningProfile": {"provisioningProfileId": "PROF1"}}},
                {"status": 200, "text": "profile-bytes"},
            ],
        ),
        run_portal_case(
            "regenerate_adhoc_profile",
            "create_or_regen_provisioning_profile",
            {
                "team_id": team,
                "profile_id": "PROF2",
                "app_id_id": "BUNDLE1",
                "profile_name": "TS com.example.app Ad Hoc",
                "certificate_ids": ["CERT2"],
                "device_ids": ["DEV1"],
                "distribution_type": "adhoc",
            },
            [
                {"status": 200, "json": {"resultCode": 0, "provisioningProfile": {"provisioningProfileId": "PROF2"}}},
                {"status": 200, "text": "profile-bytes"},
            ],
        ),
    ]


if __name__ == "__main__":
    write("python_random", python_random_cases())
    write("random_ids", random_id_cases())
    write("bundle_mapping", mapping_cases())
    with open(FIXTURES_DIR / "capabilities_response.json", "w") as f:
        json.dump(CAPABILITIES_RESPONSE, f, indent=2)
        f.write("\n")
    write("entitlements_processor", entitlements_processor_cases())
    write("entitlements_processor_apple", apple_entitlements_processor_cases())
    write("app_inspection", inspection_cases())
    write("info_plist_patching", info_plist_cases())
    write("srp", srp_cases())
    write("portal_requests", portal_cases())
