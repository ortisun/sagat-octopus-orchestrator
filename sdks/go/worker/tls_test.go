package worker_test

import (
	"bytes"
	"context"
	"crypto/tls"
	"encoding/json"
	"log/slog"
	"strings"
	"sync"
	"testing"
	"time"

	"google.golang.org/grpc/status"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/testpki"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/worker"
)

// tlsHost is a host for a TLS service that does not retry the registration: a certificate that is
// refused is refused the same way again.
func tlsHost(t *testing.T, f *fakeService, opts ...worker.Option) *worker.Host {
	t.Helper()
	opts = append([]worker.Option{worker.WithRetry(sagat.RetryPolicy{MaxAttempts: 1}), worker.WithTimeout(10 * time.Second)}, opts...)
	h, err := worker.New(f.url(), "payments", "tls-worker", sagat.StaticToken("tok"), opts...)
	if err != nil {
		t.Fatal(err)
	}
	must(t, h.RegisterActivity("charge", rawOK))
	t.Cleanup(func() {
		ctx, cancel := context.WithTimeout(context.Background(), time.Second)
		defer cancel()
		_ = h.Stop(ctx)
	})
	return h
}

func serverTLS(t *testing.T, authority *testpki.Authority, hosts ...string) *tls.Config {
	t.Helper()
	if len(hosts) == 0 {
		hosts = []string{"127.0.0.1", "localhost"}
	}
	return &tls.Config{Certificates: []tls.Certificate{authority.Server(t, "server", hosts...)}}
}

func TestAnHTTPSServiceWithACertificateNobodyTrustsIsRefusedAsATransportFailure(t *testing.T) {
	authority := testpki.NewAuthority(t, "test authority")
	f := newFakeTLSService(t, serverTLS(t, authority))
	err := tlsHost(t, f).Start(context.Background())
	var se *sagat.Error
	if !sagat.IsCode(err, sagat.CodeTransport) {
		t.Fatalf("an untrusted certificate is a transport failure: %v", err)
	}
	se = err.(*sagat.Error)
	if !se.Retryable || se.GRPCStatus != "UNAVAILABLE" {
		t.Errorf("%+v", se)
	}
	// The cause says why (a failed verification); the SDK's own message does not repeat it.
	cause := status.Convert(se.Unwrap()).Message()
	if !strings.Contains(cause, "certificate") {
		t.Errorf("the cause: %q", cause)
	}
	if strings.Contains(se.Error(), "x509") {
		t.Errorf("%q", se.Error())
	}
	if n := len(f.of("RegisterWorker")); n != 0 {
		t.Errorf("the registration reached the service %d times: the certificate was accepted", n)
	}
}

func TestTheConfiguredCAMakesTheSameServiceWorkAndATaskGoesThroughItEndToEnd(t *testing.T) {
	authority := testpki.NewAuthority(t, "test authority")
	f := newFakeTLSService(t, serverTLS(t, authority))
	h := tlsHost(t, f, worker.WithTLSConfig(&tls.Config{RootCAs: authority.Pool}))
	var input json.RawMessage
	must(t, h.RegisterActivity("echo", func(_ context.Context, in json.RawMessage) (any, error) {
		input = in
		return map[string]bool{"ok": true}, nil
	}))
	f.lease(lease(forActivity("echo", "1")))
	must(t, h.Start(context.Background()))
	until(t, "the completion over TLS", func() bool { return len(f.of("CompleteStep")) == 1 })
	if h.ProtocolVersion() != "1.2" || string(input) != `{"amount":5}` {
		t.Errorf("%q %s", h.ProtocolVersion(), input)
	}
	for _, c := range f.of("RegisterWorker") {
		if c.peer != "" {
			t.Errorf("no client certificate was configured: %q", c.peer)
		}
	}

	// A CA that did not sign the certificate does not do.
	other := testpki.NewAuthority(t, "another authority")
	g := newFakeTLSService(t, serverTLS(t, authority))
	if err := tlsHost(t, g, worker.WithTLSConfig(&tls.Config{RootCAs: other.Pool})).Start(context.Background()); !sagat.IsCode(err, sagat.CodeTransport) {
		t.Errorf("%v", err)
	}
	if len(g.of("RegisterWorker")) != 0 {
		t.Error("the registration reached the service")
	}
}

func TestTheNameOfTheServiceIsCheckedEvenWithATrustedCA(t *testing.T) {
	authority := testpki.NewAuthority(t, "test authority")
	f := newFakeTLSService(t, serverTLS(t, authority, "some-other-host.example"))
	err := tlsHost(t, f, worker.WithTLSConfig(&tls.Config{RootCAs: authority.Pool})).Start(context.Background())
	if !sagat.IsCode(err, sagat.CodeTransport) || len(f.of("RegisterWorker")) != 0 {
		t.Errorf("a certificate for another name is refused: %v", err)
	}
	// The ServerName of the configuration is the name the certificate answers to.
	named := &tls.Config{RootCAs: authority.Pool, ServerName: "some-other-host.example"}
	if err := tlsHost(t, f, worker.WithTLSConfig(named)).Start(context.Background()); err != nil {
		t.Errorf("%v", err)
	}
}

func TestMutualTLSTheServiceSeesTheWorkersCertificateAndRefusesAWorkerWithoutOne(t *testing.T) {
	authority := testpki.NewAuthority(t, "test authority")
	required := serverTLS(t, authority)
	required.ClientAuth = tls.RequireAndVerifyClientCert
	required.ClientCAs = authority.Pool
	f := newFakeTLSService(t, required)

	with := tlsHost(t, f, worker.WithTLSConfig(&tls.Config{
		RootCAs: authority.Pool, Certificates: []tls.Certificate{authority.Client(t, "payments-worker-1")},
	}))
	f.lease(lease())
	must(t, with.Start(context.Background()))
	until(t, "the task", func() bool { return len(f.of("CompleteStep")) == 1 })
	for _, method := range []string{"RegisterWorker", "PollTasks", "CompleteStep"} {
		calls := f.of(method)
		if len(calls) == 0 || calls[0].peer != "payments-worker-1" {
			t.Errorf("%s: the service saw the client certificate %q", method, calls)
		}
	}

	before := len(f.of("RegisterWorker"))
	without := tlsHost(t, f, worker.WithTLSConfig(&tls.Config{RootCAs: authority.Pool}))
	if err := without.Start(context.Background()); !sagat.IsCode(err, sagat.CodeTransport) {
		t.Errorf("a worker with no certificate is refused at the handshake: %v", err)
	}
	stranger := testpki.NewAuthority(t, "stranger")
	forged := tlsHost(t, f, worker.WithTLSConfig(&tls.Config{
		RootCAs: authority.Pool, Certificates: []tls.Certificate{stranger.Client(t, "intruder")},
	}))
	if err := forged.Start(context.Background()); !sagat.IsCode(err, sagat.CodeTransport) {
		t.Errorf("a certificate nobody trusts: %v", err)
	}
	if after := len(f.of("RegisterWorker")); after != before {
		t.Errorf("%d registrations got through without a good certificate", after-before)
	}
}

func TestNoWarningIsLoggedForHTTPS(t *testing.T) {
	authority := testpki.NewAuthority(t, "test authority")
	f := newFakeTLSService(t, serverTLS(t, authority))
	var logged bytes.Buffer
	var mu sync.Mutex
	logger := slog.New(slog.NewTextHandler(&lockedWriter{&mu, &logged}, &slog.HandlerOptions{Level: slog.LevelDebug}))
	h := tlsHost(t, f, worker.WithLogger(logger), worker.WithTLSConfig(&tls.Config{RootCAs: authority.Pool}))
	must(t, h.Start(context.Background()))
	mu.Lock()
	defer mu.Unlock()
	if strings.Contains(logged.String(), "without TLS") {
		t.Errorf("https needs no warning: %s", logged.String())
	}
	if !strings.Contains(logged.String(), "worker registered") {
		t.Errorf("the logger was used: %s", logged.String())
	}
}
