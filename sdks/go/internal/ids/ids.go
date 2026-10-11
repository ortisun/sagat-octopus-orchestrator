// Package ids makes the random identifiers the SDK needs.
package ids

import (
	"crypto/rand"
	"encoding/hex"
)

// UUID is a random (version 4) UUID in its canonical lowercase text form: the idempotency key of a
// call and the id of an event that the caller did not give.
func UUID() string {
	var b [16]byte
	if _, err := rand.Read(b[:]); err != nil {
		// crypto/rand does not fail on a supported platform; a key that is not random would break
		// idempotency, so there is nothing safe to fall back to.
		panic("sagat: no randomness available: " + err.Error())
	}
	b[6] = b[6]&0x0f | 0x40
	b[8] = b[8]&0x3f | 0x80
	var out [36]byte
	hex.Encode(out[0:8], b[0:4])
	out[8] = '-'
	hex.Encode(out[9:13], b[4:6])
	out[13] = '-'
	hex.Encode(out[14:18], b[6:8])
	out[18] = '-'
	hex.Encode(out[19:23], b[8:10])
	out[23] = '-'
	hex.Encode(out[24:], b[10:])
	return string(out[:])
}
