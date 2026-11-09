// Added to UnfairPlay by warpsign's tools/unfairplay-credential/build.sh.
//
// Exposes the 16-byte FairPlay account credential held inside a recovered
// version-2 enrollment model. That credential is the exact format used by
// fairplay-decrypt and warpsign's `decrypt --credential`: the saved model key
// XORed with 0x30 (see normalizeMacOSSINFWithModel / macOSSINFCrypt, where the
// AES key is `credential ^ 0x30`). It is a per-account secret.

package fairplay

import "fmt"

// CredentialFromModelPack returns the 16-byte account credential held in a
// version-2, selector-bound enrollment model pack.
func CredentialFromModelPack(pack []byte) ([16]byte, error) {
	model, err := parseCurrentSINFModel(pack)
	if err != nil {
		return [16]byte{}, err
	}
	if model.version != 2 || !model.hasSelector {
		return [16]byte{}, fmt.Errorf(
			"credential export needs a selector-bound version-2 enrollment model (got version %d)",
			model.version,
		)
	}
	var credential [16]byte
	for i, b := range model.key {
		credential[i] = b ^ 0x30
	}
	return credential, nil
}

// CredentialFromModelFile reads one enrollment model .bin and returns its
// 16-byte credential.
func CredentialFromModelFile(path string) ([16]byte, error) {
	pack, err := readEnrollmentModel(path)
	if err != nil {
		return [16]byte{}, err
	}
	defer clear(pack)
	return CredentialFromModelPack(pack)
}
