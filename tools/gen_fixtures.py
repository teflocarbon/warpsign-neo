"""Generate reference fixtures from the Python warpsign implementation.

warpsign-neo must reproduce Python warpsign's behaviour exactly: bundle IDs
registered by one have to come out identical from the other, or re-signed
apps lose their data. This script runs the Python code over a set of cases
and writes its outputs to fixtures/*.json for the Rust tests to compare
against.

Usage:
    python tools/gen_fixtures.py /path/to/warpsign
"""

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
from warpsign.src.ipa.entitlements_processor import EntitlementsProcessor  # noqa: E402

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


def run_mapping_case(case: dict) -> dict:
    """Mirror how SignOrchestrator drives BundleMapping during a sign"""
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


def mapping_cases() -> list:
    full_app = [
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


if __name__ == "__main__":
    write("python_random", python_random_cases())
    write("random_ids", random_id_cases())
    write("bundle_mapping", mapping_cases())
    with open(FIXTURES_DIR / "capabilities_response.json", "w") as f:
        json.dump(CAPABILITIES_RESPONSE, f, indent=2)
        f.write("\n")
    write("entitlements_processor", entitlements_processor_cases())
    write("app_inspection", inspection_cases())
