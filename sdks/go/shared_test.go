package sagat

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// repoFile returns the path of a file of the repository, or skips the test when the SDK is not tested
// from inside the repository (a module downloaded as a dependency has no repository around it).
func repoFile(t *testing.T, parts ...string) string {
	t.Helper()
	path := filepath.Join(append([]string{"..", ".."}, parts...)...)
	if _, err := os.Stat(path); err != nil {
		t.Skipf("not inside the repository (%s is missing): the copy cannot be compared", path)
	}
	return path
}

func TestSharedFilesAreTheRepositorys(t *testing.T) {
	t.Run("the embedded error catalog is the shared one", func(t *testing.T) {
		shared, err := os.ReadFile(repoFile(t, "sdks", "error-catalog.json"))
		if err != nil {
			t.Fatal(err)
		}
		if !bytes.Equal(shared, errorCatalogJSON) {
			t.Fatal("error-catalog.json is behind sdks/error-catalog.json: run scripts/generate.sh --copy-only")
		}
	})
	t.Run("the worker protocol shipped with the module is the one of the contracts", func(t *testing.T) {
		contract, err := os.ReadFile(repoFile(t, "specs", "001-platform-foundation", "contracts", "worker.proto"))
		if err != nil {
			t.Fatal(err)
		}
		shipped, err := os.ReadFile(filepath.Join("proto", "sagat_octopus_orchestrator", "v1", "worker.proto"))
		if err != nil {
			t.Fatal(err)
		}
		if !bytes.Equal(contract, shipped) {
			t.Fatal("proto/sagat_octopus_orchestrator/v1/worker.proto is behind the contract: run scripts/generate.sh")
		}
	})
	t.Run("the generated protobuf code is what the shipped proto generates", func(t *testing.T) {
		protoc, err := exec.LookPath("protoc")
		if err != nil {
			t.Skip("protoc is not installed: the generated code cannot be regenerated here (scripts/generate.sh does it)")
		}
		plugins, _ := filepath.Abs("bin")
		for _, name := range []string{"protoc-gen-go", "protoc-gen-go-grpc"} {
			if _, err := os.Stat(filepath.Join(plugins, name)); err != nil {
				t.Skipf("%s is not installed in bin/ (scripts/generate.sh installs it): not compared", name)
			}
		}
		out := t.TempDir()
		pkg := "github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/workerpb;workerpb"
		file := "sagat_octopus_orchestrator/v1/worker.proto"
		cmd := exec.Command(protoc, "--proto_path=proto",
			"--go_out="+out, "--go_opt=paths=source_relative", "--go_opt=M"+file+"="+pkg,
			"--go-grpc_out="+out, "--go-grpc_opt=paths=source_relative", "--go-grpc_opt=M"+file+"="+pkg, file)
		cmd.Env = append(os.Environ(), "PATH="+plugins+string(os.PathListSeparator)+os.Getenv("PATH"))
		if output, err := cmd.CombinedOutput(); err != nil {
			t.Fatalf("protoc failed: %v\n%s", err, output)
		}
		for _, name := range []string{"worker.pb.go", "worker_grpc.pb.go"} {
			fresh, err := os.ReadFile(filepath.Join(out, "sagat_octopus_orchestrator", "v1", name))
			if err != nil {
				t.Fatal(err)
			}
			committed, err := os.ReadFile(filepath.Join("internal", "workerpb", name))
			if err != nil {
				t.Fatal(err)
			}
			if withoutVersions(fresh) != withoutVersions(committed) {
				t.Fatalf("internal/workerpb/%s is behind the proto: run scripts/generate.sh", name)
			}
		}
	})
}

// withoutVersions drops the header lines that record the version of protoc, which differs between
// machines and does not change the code.
func withoutVersions(code []byte) string {
	var kept []string
	for _, line := range strings.Split(string(code), "\n") {
		if strings.HasPrefix(line, "// \tprotoc ") || strings.HasPrefix(line, "// \tprotoc\t") {
			continue
		}
		kept = append(kept, line)
	}
	return strings.Join(kept, "\n")
}

func TestEveryErrorHasARemediationADefaultAndACodeOfItsOwn(t *testing.T) {
	loadCatalog()
	known := map[Code]bool{
		CodeBadRequest: true, CodeUnauthenticated: true, CodeForbidden: true, CodeNotFound: true, CodeConflict: true,
		CodeValidationFailed: true, CodeRateLimited: true, CodeQuotaExceeded: true, CodeUnavailable: true, CodeInternal: true,
		CodeStaleAttempt: true, CodeIdempotencyKeyReused: true, CodeNotBound: true, CodeSecretNotUsable: true,
		CodeUnsupportedProtocol: true, CodeTransport: true, CodeCancelled: true, CodeProtocol: true, CodePayloadTooLarge: true,
	}
	if len(catalogEntries) != len(known) {
		t.Fatalf("the catalog has %d codes and the SDK %d constants", len(catalogEntries), len(known))
	}
	seen := map[Code]bool{}
	for _, entry := range catalogEntries {
		if seen[entry.Code] {
			t.Errorf("%s twice", entry.Code)
		}
		seen[entry.Code] = true
		if !known[entry.Code] {
			t.Errorf("%s has no constant", entry.Code)
		}
		if len(entry.Remediation) <= 20 {
			t.Errorf("%s has no remediation", entry.Code)
		}
		if Remediation(entry.Code) != entry.Remediation {
			t.Errorf("%s: remediation differs from the catalog", entry.Code)
		}
		if RetryableByDefault(entry.Code) != entry.Retryable {
			t.Errorf("%s: retryable differs from the catalog", entry.Code)
		}
	}
}

func TestAnErrorCarriesTheCodeTheTraceAndTheRemediationAndSaysNothingElse(t *testing.T) {
	retryAfter := 2 * time.Second
	secret := errors.New("Bearer sk_live_must_not_appear")
	e := NewError(CodeRateLimited, "slow down")
	e.HTTPStatus, e.TraceID, e.Reason, e.RetryAfter = 429, strings.Repeat("a", 32), "TENANT", &retryAfter
	e.WithCause(secret)
	if !e.Retryable {
		t.Error("RATE_LIMITED is retryable by default")
	}
	if e.Remediation != Remediation(CodeRateLimited) {
		t.Error("the remediation is the catalog's")
	}
	text := e.Error()
	for _, want := range []string{"RATE_LIMITED", "slow down", strings.Repeat("a", 32)} {
		if !strings.Contains(text, want) {
			t.Errorf("%q is missing from %q", want, text)
		}
	}
	if strings.Contains(text, "sk_live") {
		t.Errorf("the cause leaked into the message: %q", text)
	}
	var wrapped error = e
	var target *Error
	if !errors.As(wrapped, &target) || target != e {
		t.Error("errors.As does not reach the error")
	}
	if !errors.Is(wrapped, secret) {
		t.Error("errors.Is does not reach the cause")
	}
	if !IsCode(wrapped, CodeRateLimited) || IsCode(wrapped, CodeNotFound) {
		t.Error("IsCode")
	}
	if got := Errorf(CodeNotFound, "no %s", "run").Message; got != "no run" {
		t.Errorf("Errorf: %q", got)
	}
	if Invalid("x").Code != CodeValidationFailed {
		t.Error("Invalid is VALIDATION_FAILED")
	}
}

func TestAStatusWithoutAnEnvelopeIsClassifiedByTheCatalog(t *testing.T) {
	cases := map[int]Code{
		400: CodeBadRequest, 401: CodeUnauthenticated, 403: CodeForbidden, 404: CodeNotFound, 409: CodeConflict, 422: CodeValidationFailed,
		429: CodeRateLimited, 502: CodeUnavailable, 503: CodeUnavailable, 504: CodeUnavailable, 500: CodeInternal, 418: CodeBadRequest, 501: CodeInternal,
	}
	for status, want := range cases {
		if got := CodeOfHTTPStatus(status); got != want {
			t.Errorf("%d: %s, want %s", status, got, want)
		}
	}
}

func TestTraceparent(t *testing.T) {
	valid := "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01"
	if !ValidTraceparent(valid) || CheckTraceparent(valid) != nil {
		t.Fatal("a valid traceparent was refused")
	}
	if got := TraceIDOf(valid); got != "0af7651916cd43dd8448eb211c80319c" {
		t.Errorf("trace id %q", got)
	}
	for _, bad := range []string{
		"", "not-a-trace", "00-0AF7651916CD43DD8448EB211C80319C-b7ad6b7169203331-01", // upper case
		"01-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01", // another version
		"00-00000000000000000000000000000000-b7ad6b7169203331-01", // zero trace id
		"00-0af7651916cd43dd8448eb211c80319c-0000000000000000-01", // zero parent id
		"00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-0",  // short flags
		valid + "-extra",
	} {
		if ValidTraceparent(bad) {
			t.Errorf("%q was accepted", bad)
		}
	}
	if !IsCode(CheckTraceparent("nope"), CodeBadRequest) {
		t.Error("a malformed traceparent is BAD_REQUEST")
	}
	if fresh := NewTraceparent(); !ValidTraceparent(fresh) || fresh == NewTraceparent() {
		t.Errorf("a fresh trace context is valid and different each time: %q", fresh)
	}
}

func TestTokenSources(t *testing.T) {
	ctx := context.Background()
	token, err := BearerToken(ctx, StaticToken("tok-1"))
	if err != nil || token != "tok-1" {
		t.Fatalf("static: %q %v", token, err)
	}
	if CanRefresh(StaticToken("x")) {
		t.Error("a static token cannot be refreshed")
	}
	n := 0
	provider := TokenFunc(func(context.Context) (string, error) { n++; return "tok-" + string(rune('0'+n)), nil })
	if !CanRefresh(provider) {
		t.Error("a provider can be asked again")
	}
	first, _ := BearerToken(ctx, provider)
	second, _ := BearerToken(ctx, provider)
	if first == second {
		t.Errorf("the provider is asked before every use: %q %q", first, second)
	}
	failing := TokenFunc(func(context.Context) (string, error) { return "", errors.New("vault said: secret-detail") })
	_, err = BearerToken(ctx, failing)
	if !IsCode(err, CodeUnauthenticated) || strings.Contains(err.Error(), "secret-detail") {
		t.Errorf("a failing provider is UNAUTHENTICATED without its words: %v", err)
	}
	if _, err := BearerToken(ctx, nil); !IsCode(err, CodeUnauthenticated) {
		t.Errorf("no source: %v", err)
	}
	cancelled, cancel := context.WithCancel(ctx)
	cancel()
	_, err = BearerToken(cancelled, failing)
	if !IsCode(err, CodeCancelled) || !errors.Is(err, context.Canceled) {
		t.Errorf("a cancelled call is CANCELLED: %v", err)
	}
}

func TestRetryPolicyKeepsDefaultsForZeroFields(t *testing.T) {
	got := RetryPolicy{MaxAttempts: 1}.WithDefaults()
	if got.MaxAttempts != 1 || got.InitialDelay != DefaultRetry.InitialDelay || got.MaxDelay != DefaultRetry.MaxDelay {
		t.Errorf("%+v", got)
	}
	if (RetryPolicy{}).WithDefaults() != DefaultRetry {
		t.Error("the zero policy is the default one")
	}
}

func TestVersionConstants(t *testing.T) {
	if Version != "0.1.0" || ProtocolVersion != "1.4" || Language != "go" {
		t.Errorf("%s %s %s", Version, ProtocolVersion, Language)
	}
	var file struct {
		Version int `json:"version"`
	}
	if err := json.Unmarshal(errorCatalogJSON, &file); err != nil || file.Version != 1 {
		t.Errorf("the catalog is version 1: %v %d", err, file.Version)
	}
}

// Certificate verification is never switched off by the SDK: not for a test, not for development, not
// by an option. A server certificate nobody trusts is a TRANSPORT failure (client/tls_test.go,
// worker/tls_test.go); this keeps the words that would turn that off out of the library.
func TestNoSDKCodeSkipsTheVerificationOfCertificates(t *testing.T) {
	checked := 0
	err := filepath.WalkDir(".", func(path string, entry os.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if entry.IsDir() {
			if entry.Name() == "bin" {
				return filepath.SkipDir
			}
			return nil
		}
		if !strings.HasSuffix(path, ".go") || strings.HasSuffix(path, "_test.go") {
			return nil
		}
		source, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		checked++
		for _, forbidden := range []string{"InsecureSkipVerify", "VerifyPeerCertificate", "VerifyConnection"} {
			if strings.Contains(string(source), forbidden) {
				t.Errorf("%s mentions %s", path, forbidden)
			}
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	if checked < 20 {
		t.Errorf("only %d source files were looked at", checked)
	}
}
