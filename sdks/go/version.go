package sagat

// Version is the version of this SDK. It is sent in the User-Agent of every REST call as
// sagat-sdk-go/<Version>.
const Version = "0.1.0"

// ProtocolVersion is the newest minor version of worker protocol 1 this SDK speaks (sdks/SPEC.md,
// section 6.1). The server answers with the newest version both speak.
const ProtocolVersion = "1.2"

// Language is the language the worker host announces in its capabilities.
const Language = "go"
