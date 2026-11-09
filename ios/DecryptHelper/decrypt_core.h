#ifndef DECRYPT_CORE_H
#define DECRYPT_CORE_H

#include <stddef.h>

// Decrypt one FairPlay-encrypted Mach-O. `src_path` is the binary inside the
// helper's bundle (with its SC_Info/<name>.sinf replicated alongside);
// `dst_path` is a writable copy of it. On success the decrypted pages are
// written into `dst_path` and its cryptid is cleared. `msg` receives a short
// human-readable status.
//
// Returns 1 = decrypted, 0 = nothing to do (not encrypted / not an arm64
// Mach-O), -1 = error (see `msg`).
int dh_decrypt_into(const char *src_path, const char *dst_path, char *msg, size_t msglen);

#endif
