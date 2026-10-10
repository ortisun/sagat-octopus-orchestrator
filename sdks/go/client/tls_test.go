package client_test

import (
	"bytes"
	"context"
	"crypto/tls"
	"crypto/x509"
	"errors"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/client"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/testpki"
)

// tlsFake is an https server with a certificate of its own authority. It records the client
// certificates it saw.
type tlsFake struct {
	server *httptest.Server
	mu     sync.Mutex
	calls  int
	peers  []string // the common name of the client certificate of each request, "" for none
}

func newTLSFake(t *testing.T, serverTLS *tls.Config) *tlsFake {
	f := &tlsFake{}
	f.server = httptest.NewUnstartedServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		f.mu.Lock()
		f.calls++
		peer := ""
		if r.TLS != nil && len(r.TLS.PeerCertificates) > 0 {
			peer = r.TLS.PeerCertificates[0].Subject.CommonName
		}
		f.peers = append(f.peers, peer)
		f.mu.Unlock()
		reply(w, 200, run(nil))
	}))
	f.server.TLS = serverTLS
	f.server.Config.ErrorLog = nil
	f.server.StartTLS()
	t.Cleanup(f.server.Close)
	return f
}

func (f *tlsFake) base() string { return f.server.URL + "/v1" }

func (f *tlsFake) seen() (int, []string) {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.calls, append([]string(nil), f.peers...)
}

// once is a client that does not retry: a certificate that is refused is refused the same way again.
func once(t *testing.T, base string, opts ...client.Option) *client.Client {
	t.Helper()
	opts = append([]client.Option{client.WithRetry(sagat.RetryPolicy{MaxAttempts: 1}), client.WithTimeout(10 * time.Second)}, opts...)
	c, err := client.New(base, "payments", sagat.StaticToken("tok-1"), opts...)
	if err != nil {
		t.Fatal(err)
	}
	return c
}

func TestHTTPSWithTheSystemRootsRefusesAServerCertificateNobodyTrusts(t *testing.T) {
	authority := testpki.NewAuthority(t, "test authority")
	f := newTLSFake(t, &tls.Config{Certificates: []tls.Certificate{authority.Server(t, "server", "127.0.0.1", "localhost")}})
	_, err := once(t, f.base()).Get(context.Background(), "r")
	se := asSagat(t, err)
	if se.Code != sagat.CodeTransport || !se.Retryable {
		t.Errorf("an untrusted certificate is a transport failure: %+v", se)
	}
	// What the failure is called depends on the platform's verifier; that it is a failed verification
	// does not.
	var verification *tls.CertificateVerificationError
	if !errors.As(err, &verification) {
		t.Errorf("the cause is the verification failure: %v", errors.Unwrap(err))
	}
	if strings.Contains(se.Error(), "x509") || strings.Contains(se.Message, "127.0.0.1") {
		t.Errorf("the message is the SDK's own words: %q", se.Error())
	}
	if calls, _ := f.seen(); calls != 0 {
		t.Errorf("the request reached the server %d times: the certificate was accepted", calls)
	}
}

func TestTheConfiguredCAMakesTheSameServerWork(t *testing.T) {
	authority := testpki.NewAuthority(t, "test authority")
	f := newTLSFake(t, &tls.Config{Certificates: []tls.Certificate{authority.Server(t, "server", "127.0.0.1", "localhost")}})
	c := once(t, f.base(), client.WithTLSConfig(&tls.Config{RootCAs: authority.Pool}))
	got, err := c.Get(context.Background(), "r")
	if err != nil || got.Status != client.StatusRunning {
		t.Fatalf("%v %v", got, err)
	}
	if calls, peers := f.seen(); calls != 1 || peers[0] != "" {
		t.Errorf("%d calls, client certificates %q", calls, peers)
	}
	// Another authority's roots do not trust it.
	other := testpki.NewAuthority(t, "another authority")
	_, err = once(t, f.base(), client.WithTLSConfig(&tls.Config{RootCAs: other.Pool})).Get(context.Background(), "r")
	if !sagat.IsCode(err, sagat.CodeTransport) {
		t.Errorf("a CA that did not sign it: %v", err)
	}
}

func TestTheNameOfTheServerIsCheckedEvenWithATrustedCA(t *testing.T) {
	authority := testpki.NewAuthority(t, "test authority")
	f := newTLSFake(t, &tls.Config{Certificates: []tls.Certificate{authority.Server(t, "server", "some-other-host.example")}})
	_, err := once(t, f.base(), client.WithTLSConfig(&tls.Config{RootCAs: authority.Pool})).Get(context.Background(), "r")
	var wrongName x509.HostnameError
	if !sagat.IsCode(err, sagat.CodeTransport) || !errors.As(err, &wrongName) {
		t.Errorf("a certificate for another name is refused: %v", err)
	}
	if calls, _ := f.seen(); calls != 0 {
		t.Errorf("%d requests reached the server", calls)
	}
	// ServerName of the configuration is honoured: that is the name the certificate answers to.
	named := &tls.Config{RootCAs: authority.Pool, ServerName: "some-other-host.example"}
	if _, err := once(t, f.base(), client.WithTLSConfig(named)).Get(context.Background(), "r"); err != nil {
		t.Errorf("%v", err)
	}
}

func TestMutualTLSTheServerSeesTheClientCertificateAndRefusesAClientWithoutOne(t *testing.T) {
	authority := testpki.NewAuthority(t, "test authority")
	f := newTLSFake(t, &tls.Config{
		Certificates: []tls.Certificate{authority.Server(t, "server", "127.0.0.1", "localhost")},
		ClientAuth:   tls.RequireAndVerifyClientCert,
		ClientCAs:    authority.Pool,
	})
	withCert := once(t, f.base(), client.WithTLSConfig(&tls.Config{
		RootCAs: authority.Pool, Certificates: []tls.Certificate{authority.Client(t, "worker-client-1")},
	}))
	if _, err := withCert.Get(context.Background(), "r"); err != nil {
		t.Fatal(err)
	}
	if calls, peers := f.seen(); calls != 1 || peers[0] != "worker-client-1" {
		t.Errorf("the server saw %d calls and the client certificates %q", calls, peers)
	}

	without := once(t, f.base(), client.WithTLSConfig(&tls.Config{RootCAs: authority.Pool}))
	_, err := without.Get(context.Background(), "r")
	if se := asSagat(t, err); se.Code != sagat.CodeTransport {
		t.Errorf("a client with no certificate is refused at the handshake: %+v", se)
	}
	// A certificate of another authority is refused as well.
	stranger := testpki.NewAuthority(t, "stranger")
	forged := once(t, f.base(), client.WithTLSConfig(&tls.Config{
		RootCAs: authority.Pool, Certificates: []tls.Certificate{stranger.Client(t, "intruder")},
	}))
	if _, err := forged.Get(context.Background(), "r"); !sagat.IsCode(err, sagat.CodeTransport) {
		t.Errorf("a client certificate nobody trusts: %v", err)
	}
	if calls, _ := f.seen(); calls != 1 {
		t.Errorf("only the first client reached the handler: %d calls", calls)
	}
}

func TestNoWarningIsLoggedForHTTPSAndTheWarningForPlainHTTPIsStillThere(t *testing.T) {
	var logged bytes.Buffer
	logger := slog.New(slog.NewTextHandler(&logged, nil))
	if _, err := client.New("https://api.example.com/v1", "ns", sagat.StaticToken("t"), client.WithLogger(logger)); err != nil {
		t.Fatal(err)
	}
	if logged.Len() != 0 {
		t.Errorf("https needs no warning: %s", logged.String())
	}
	if _, err := client.New("http://api.example.com/v1", "ns", sagat.StaticToken("t"), client.WithLogger(logger)); err != nil {
		t.Fatal(err)
	}
	if strings.Count(logged.String(), "without TLS") != 1 {
		t.Errorf("%s", logged.String())
	}
}

func TestTheTLSConfigurationIsCopiedSoLaterChangesToItDoNotReachTheClient(t *testing.T) {
	authority := testpki.NewAuthority(t, "test authority")
	f := newTLSFake(t, &tls.Config{Certificates: []tls.Certificate{authority.Server(t, "server", "127.0.0.1")}})
	config := &tls.Config{RootCAs: authority.Pool}
	c := once(t, f.base(), client.WithTLSConfig(config))
	config.RootCAs = x509.NewCertPool() // the application changes its mind: the client has its own copy
	if _, err := c.Get(context.Background(), "r"); err != nil {
		t.Errorf("%v", err)
	}
}
