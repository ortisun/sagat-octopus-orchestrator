// cacheprog exposes the image's read-only Go build cache and keeps per-build
// additions in /tmp. It implements the documented GOCACHEPROG JSON protocol.
package main

import (
	"bufio"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
)

const (
	maxCacheBody    = 24 * 1024 * 1024
	maxCacheMessage = 33 * 1024 * 1024 // base64 plus JSON quotes and newline
)

type request struct {
	ID       int64
	Command  string
	ActionID []byte
	OutputID []byte
	BodySize int64
}

type response struct {
	ID            int64    `json:",omitempty"`
	Err           string   `json:",omitempty"`
	KnownCommands []string `json:",omitempty"`
	Miss          bool     `json:",omitempty"`
	OutputID      []byte   `json:",omitempty"`
	Size          int64    `json:",omitempty"`
	DiskPath      string   `json:",omitempty"`
}

type entry struct {
	output []byte
	size   int64
	path   string
}

func send(encoder *json.Encoder, value response) {
	if err := encoder.Encode(value); err != nil {
		os.Exit(1)
	}
}

func cachePath(root string, digest []byte, suffix string) (string, error) {
	if len(digest) != 32 {
		return "", errors.New("cache digest has the wrong length")
	}
	encoded := hex.EncodeToString(digest)
	return filepath.Join(root, encoded[:2], encoded+suffix), nil
}

func seedEntry(root string, action []byte) (entry, bool) {
	path, err := cachePath(root, action, "-a")
	if err != nil {
		return entry{}, false
	}
	contents, err := os.ReadFile(path)
	if err != nil {
		return entry{}, false
	}
	fields := strings.Fields(string(contents))
	if len(fields) != 5 || fields[0] != "v1" || fields[1] != hex.EncodeToString(action) {
		return entry{}, false
	}
	output, err := hex.DecodeString(fields[2])
	if err != nil || len(output) != 32 {
		return entry{}, false
	}
	size, err := strconv.ParseInt(fields[3], 10, 64)
	if err != nil || size < 0 {
		return entry{}, false
	}
	dataPath, err := cachePath(root, output, "-d")
	if err != nil {
		return entry{}, false
	}
	info, err := os.Stat(dataPath)
	if err != nil || info.Size() != size || !info.Mode().IsRegular() {
		return entry{}, false
	}
	return entry{output: output, size: size, path: dataPath}, true
}

func main() {
	if len(os.Args) != 2 {
		os.Exit(2)
	}
	seedRoot := os.Args[1]
	writableRoot := "/tmp/sagat-go-cache"
	if err := os.MkdirAll(writableRoot, 0o700); err != nil {
		os.Exit(1)
	}

	scanner := bufio.NewScanner(os.Stdin)
	scanner.Buffer(make([]byte, 4096), maxCacheMessage)
	encoder := json.NewEncoder(os.Stdout)
	send(encoder, response{KnownCommands: []string{"get", "put", "close"}})
	writes := make(map[string]entry)

	for scanner.Scan() {
		if len(scanner.Bytes()) == 0 {
			continue
		}
		var input request
		if err := json.Unmarshal(scanner.Bytes(), &input); err != nil {
			fmt.Fprintln(os.Stderr, "cache request:", err)
			os.Exit(1)
		}
		actionKey := hex.EncodeToString(input.ActionID)
		switch input.Command {
		case "get":
			value, ok := writes[actionKey]
			if !ok {
				value, ok = seedEntry(seedRoot, input.ActionID)
			}
			if !ok {
				send(encoder, response{ID: input.ID, Miss: true})
				continue
			}
			send(encoder, response{ID: input.ID, OutputID: value.output, Size: value.size, DiskPath: value.path})
		case "put":
			if input.BodySize < 0 || input.BodySize > maxCacheBody {
				send(encoder, response{ID: input.ID, Err: "cache body is missing or too large"})
				continue
			}
			var body []byte
			if input.BodySize > 0 {
				if !scanner.Scan() {
					send(encoder, response{ID: input.ID, Err: "cache body is missing or too large"})
					continue
				}
				for len(scanner.Bytes()) == 0 && scanner.Scan() {
				}
				if err := json.Unmarshal(scanner.Bytes(), &body); err != nil || int64(len(body)) != input.BodySize {
					send(encoder, response{ID: input.ID, Err: "cache body does not match its declared size"})
					continue
				}
			}
			if len(input.ActionID) != 32 || len(input.OutputID) != 32 {
				send(encoder, response{ID: input.ID, Err: "cache digest has the wrong length"})
				continue
			}
			path := filepath.Join(writableRoot, fmt.Sprintf("%x-d", input.OutputID))
			if err := os.WriteFile(path, body, 0o600); err != nil {
				send(encoder, response{ID: input.ID, Err: err.Error()})
				continue
			}
			writes[actionKey] = entry{output: input.OutputID, size: input.BodySize, path: path}
			send(encoder, response{ID: input.ID, DiskPath: path})
		case "close":
			send(encoder, response{ID: input.ID})
			return
		default:
			send(encoder, response{ID: input.ID, Err: "unsupported cache command"})
		}
	}
	if err := scanner.Err(); err != nil {
		fmt.Fprintln(os.Stderr, "cache protocol:", err)
		os.Exit(1)
	}
}
