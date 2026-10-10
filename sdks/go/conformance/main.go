// The conformance runner of the Go SDK (tests/contract/sdk-conformance/README.md). It uses only the
// public API of the SDK: the builders in "build" mode, the client and the worker host in "run" mode,
// and plain HTTP for the control endpoint.
//
//	runner build <scenario.json>
//	runner run <scenario.json> <config.json> <out.json>
package main

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"strings"
	"time"
)

// object is a decoded JSON object. Numbers are json.Number, so an integer is never turned into a
// float and 600000 stays 600000.
type object = map[string]any

func decode(data []byte) (any, error) {
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.UseNumber()
	var value any
	if err := decoder.Decode(&value); err != nil {
		return nil, err
	}
	return value, nil
}

func readObject(path string) (object, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	value, err := decode(data)
	if err != nil {
		return nil, fmt.Errorf("%s: %w", path, err)
	}
	obj, ok := value.(object)
	if !ok {
		return nil, fmt.Errorf("%s is not a JSON object", path)
	}
	return obj, nil
}

// encode writes JSON without HTML escaping: what the scenario said, as it said it.
func encode(value any) ([]byte, error) {
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	if err := enc.Encode(value); err != nil {
		return nil, err
	}
	return bytes.TrimRight(buf.Bytes(), "\n"), nil
}

// raw is the JSON text of a value of the scenario, to hand to the SDK as it is.
func raw(value any) json.RawMessage {
	data, err := encode(value)
	if err != nil {
		panic(err)
	}
	return data
}

// templated replaces {{unique}} in every string of the scenario.
func templated(value any, unique string) any {
	switch v := value.(type) {
	case string:
		return strings.ReplaceAll(v, "{{unique}}", unique)
	case []any:
		out := make([]any, len(v))
		for i, item := range v {
			out[i] = templated(item, unique)
		}
		return out
	case object:
		out := make(object, len(v))
		for key, item := range v {
			out[key] = templated(item, unique)
		}
		return out
	default:
		return value
	}
}

func main() {
	// A scenario that waits for something that never comes must fail, not hang or pass.
	watchdog := time.AfterFunc(10*time.Minute, func() {
		fmt.Fprintln(os.Stderr, "the runner took more than 10 minutes: giving up")
		os.Exit(1)
	})
	defer watchdog.Stop()
	if err := mainErr(os.Args[1:]); err != nil {
		fmt.Fprintln(os.Stderr, "runner:", err)
		os.Exit(1)
	}
}

func mainErr(args []string) error {
	if len(args) < 2 {
		return errors.New("usage: runner build <scenario> | run <scenario> <config> <out>")
	}
	scenario, err := readObject(args[1])
	if err != nil {
		return err
	}
	switch args[0] {
	case "build":
		out, err := build(scenario)
		if err != nil {
			return err
		}
		data, err := encode(out)
		if err != nil {
			return err
		}
		_, err = os.Stdout.Write(data)
		return err
	case "run":
		if len(args) < 4 {
			return errors.New("usage: runner run <scenario> <config> <out>")
		}
		config, err := readObject(args[2])
		if err != nil {
			return err
		}
		unique, _ := config["unique"].(string)
		out, err := run(templated(scenario, unique).(object), config)
		if err != nil {
			return err
		}
		data, err := encode(out)
		if err != nil {
			return err
		}
		// The file is written only when the scenario was played to its end.
		return os.WriteFile(args[3], data, 0o644)
	default:
		return fmt.Errorf("unknown mode %q", args[0])
	}
}
