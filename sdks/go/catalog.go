package sagat

import (
	_ "embed"
	"encoding/json"
	"sync"
)

// errorCatalogJSON is a copy of sdks/error-catalog.json, made by scripts/generate.sh. A unit test
// proves that the copy equals the file of the repository.
//
//go:embed error-catalog.json
var errorCatalogJSON []byte

type catalogEntry struct {
	Code        Code   `json:"code"`
	Origin      string `json:"origin"`
	HTTP        []int  `json:"http"`
	GRPC        string `json:"grpc"`
	Retryable   bool   `json:"retryable"`
	Remediation string `json:"remediation"`
}

var (
	catalogOnce    sync.Once
	catalogEntries []catalogEntry
	catalogByCode  map[Code]catalogEntry
)

func loadCatalog() {
	catalogOnce.Do(func() {
		var file struct {
			Errors []catalogEntry `json:"errors"`
		}
		// The file is embedded at build time and checked by a test: a failure here is a broken build.
		if err := json.Unmarshal(errorCatalogJSON, &file); err != nil {
			panic("sagat: the embedded error catalog is not valid JSON: " + err.Error())
		}
		catalogEntries = file.Errors
		catalogByCode = make(map[Code]catalogEntry, len(file.Errors))
		for _, entry := range file.Errors {
			catalogByCode[entry.Code] = entry
		}
	})
}

// Remediation is what to do about a code: the fixed sentence of the error catalog.
func Remediation(code Code) string {
	loadCatalog()
	if entry, ok := catalogByCode[code]; ok {
		return entry.Remediation
	}
	return "Report the trace id to the operator."
}

// RetryableByDefault is the catalog's value of Error.Retryable for a code, for an error the server
// did not classify itself.
func RetryableByDefault(code Code) bool {
	loadCatalog()
	return catalogByCode[code].Retryable
}

// CodeOfHTTPStatus is the code the catalog gives an HTTP status when the body carried no error
// envelope.
func CodeOfHTTPStatus(status int) Code {
	loadCatalog()
	for _, entry := range catalogEntries {
		if entry.Origin != "rest" {
			continue
		}
		for _, s := range entry.HTTP {
			if s == status {
				return entry.Code
			}
		}
	}
	if status >= 500 {
		return CodeInternal
	}
	return CodeBadRequest
}
