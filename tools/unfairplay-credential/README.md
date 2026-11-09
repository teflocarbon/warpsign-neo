# unfairplay-credential

Get the 16-byte FairPlay **account credential** from an enrolled Mac (no
jailbreak), so `warpsign decrypt --credential` / `warpsign sign --credential`
can decrypt App Store apps you own.

[UnfairPlay](https://github.com/DrWhax/unfairplay) recovers the credential into
its enrollment model but has no command to export the bare 16 bytes. This adds
one: `build.sh` fetches UnfairPlay pinned to a known commit, drops in two small
files, registers an `export-credential` subcommand automatically (no hand-
editing), and builds it. The credential it writes is byte-for-byte the format
`warpsign` expects (verified: UnfairPlay's AES key is `credential ^ 0x30` with
an `md5(name+iviv)` IV and the same `itun`/`key `/`iviv` markers as
fairplay-decrypt's `derive_work16`).

> ⚠️ The added Go is **not compiled or tested in CI** (it reaches into
> UnfairPlay's internals, on macOS). Build it on your Mac and run the validation
> step before trusting a credential. It assumes a selector-bound **version-2**
> model (what `setup` produces) and errors clearly otherwise.

## Requirements

- An **Apple Silicon Mac** (UnfairPlay's `setup` installs an iPhone/iPad app
  from the Mac App Store), signed into the Apple ID that owns the apps.
- `go` and `git` (`brew install go git`). Optional: `ipatool` to download IPAs.

## Use

```sh
cd tools/unfairplay-credential
./build.sh
```
Then:
```sh
sudo ./unfairplay setup --app /Applications/App.app --ipa App.ipa
```
```sh
./unfairplay export-credential        # -> credential.bin (0600)
```
```sh
warpsign decrypt App.ipa App-decrypted.ipa --credential credential.bin
```

## Validate (recommended once)

Decrypt the same IPA both ways and compare the executable bytes (the IPA zips
differ; the Mach-O must not):

```sh
./unfairplay decrypt --input App.ipa
warpsign decrypt App.ipa App.warp.ipa --credential credential.bin
unzip -p App.decrypted.ipa 'Payload/*.app/*' | shasum -a 256
unzip -p App.warp.ipa      'Payload/*.app/*' | shasum -a 256
```
Matching hashes ⇒ the credential is correct.

## Notes

- `build.sh` is pinned to a specific UnfairPlay commit for reproducibility; bump
  `PIN` in it to build a newer version.
- `credential.bin` is a per-account secret — keep it private, never commit it
  (`.gitignore` already excludes it, the `src/` clone and the built binary).
- One credential decrypts every app you own under that Apple ID.
