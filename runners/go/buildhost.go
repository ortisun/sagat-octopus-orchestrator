// The trusted Go build host runs one fixed `go build`, never an author's project command.
package main

import (
	"bytes"
	"compress/gzip"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"time"
)

const module = "module sagat.invalid/inline\n\ngo 1.25.3\n"
const wrapper = `package main

import (
	"encoding/json"
	"os"
	"sagat.invalid/inline/octopus"
)

func main() {
	channel, err := octopus.Channel()
	if err != nil { panic(err) }
	var input any
	if err := json.NewDecoder(os.NewFile(3, "input.json")).Decode(&input); err != nil { panic(err) }
	client := octopus.New(channel, os.Stdin, os.Getenv("SAGAT_IDEMPOTENCY_KEY"))
	value, err := Run(input, client)
	if err != nil { client.Fail(err); os.Exit(1) }
	client.Result(value)
}
`

type request struct {
	Op       string            `json:"op"`
	Source   string            `json:"source"`
	Lockfile *string           `json:"lockfile"`
	Packages []json.RawMessage `json:"packages"`
}

func say(value any) {
	bytes, _ := json.Marshal(value)
	_, _ = os.Stdout.Write(append(bytes, '\n'))
}

func write(path, contents string) error {
	return os.WriteFile(path, []byte(contents), 0o600)
}

func canonicalGzip(plain []byte) ([]byte, error) {
	var compressed bytes.Buffer
	writer, err := gzip.NewWriterLevel(&compressed, gzip.BestCompression)
	if err != nil {
		return nil, err
	}
	writer.Header.ModTime = time.Unix(0, 0)
	writer.Header.OS = 3
	if _, err = writer.Write(plain); err != nil {
		return nil, err
	}
	if err = writer.Close(); err != nil {
		return nil, err
	}
	return compressed.Bytes(), nil
}

func build() error {
	decoder := json.NewDecoder(io.LimitReader(os.Stdin, 2*1024*1024))
	decoder.DisallowUnknownFields()
	var input request
	if err := decoder.Decode(&input); err != nil {
		return errors.New("the Go build request is not valid JSON")
	}
	if input.Op != "build" || input.Source == "" {
		return errors.New("the Go build request has another shape")
	}
	if input.Lockfile != nil && *input.Lockfile != "" || len(input.Packages) != 0 {
		return errors.New("Go inline builds do not accept a lockfile or packages")
	}
	root := "/tmp/sagat-build"
	if err := os.MkdirAll(filepath.Join(root, "octopus"), 0o700); err != nil {
		return err
	}
	if err := write(filepath.Join(root, "go.mod"), module); err != nil {
		return err
	}
	if err := write(filepath.Join(root, "script.go"), input.Source); err != nil {
		return err
	}
	if err := write(filepath.Join(root, "main.go"), wrapper); err != nil {
		return err
	}
	trusted, err := os.ReadFile("/opt/sagat/trusted/octopus.go")
	if err != nil {
		return err
	}
	if err = os.WriteFile(filepath.Join(root, "octopus", "octopus.go"), trusted, 0o400); err != nil {
		return err
	}

	command := exec.Command(
		"go", "build", "-trimpath", "-buildvcs=false",
		"-ldflags=-buildid= -s -w", "-o", "/tmp/script.elf", ".",
	)
	command.Dir = root
	command.Env = []string{
		"PATH=/usr/local/go/bin:/usr/bin:/bin", "HOME=/tmp", "CGO_ENABLED=0", "GOOS=linux",
		"GOARCH=" + runtime.GOARCH, "GOAMD64=v1", "GOTOOLCHAIN=local", "GOPROXY=off", "GOSUMDB=off",
		"GOWORK=off", "GOFLAGS=-mod=readonly", "GOCACHE=/tmp/go-cache",
		"GOCACHEPROG=/opt/sagat/cacheprog /opt/sagat/go-cache",
		"GOMODCACHE=/tmp/go-mod", "TMPDIR=/tmp",
	}
	var diagnostic bytes.Buffer
	command.Stderr = &diagnostic
	if err = command.Run(); err != nil {
		_, _ = os.Stderr.Write(diagnostic.Bytes())
		first := strings.SplitN(diagnostic.String(), "\n", 2)[0]
		if first == "" {
			first = "go build failed"
		}
		return errors.New(first)
	}
	program, err := os.ReadFile("/tmp/script.elf")
	if err != nil {
		return err
	}
	compressed, err := canonicalGzip(program)
	if err != nil {
		return err
	}
	say(map[string]any{
		"op": "built", "entry": "script.elf",
		"files": map[string]string{"script.elf": base64.StdEncoding.EncodeToString(compressed)},
	})
	return nil
}

func main() {
	say(map[string]string{"op": "ready"})
	if err := build(); err != nil {
		say(map[string]string{"op": "rejected", "message": fmt.Sprint(err)})
		os.Exit(1)
	}
}
