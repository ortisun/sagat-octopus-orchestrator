// Package client is the REST client of the Sagat Octopus Orchestrator (sdks/SPEC.md, section 3):
// start, read, list, operate, cancel and signal runs, and publish events.
//
// A Client is safe for concurrent use. Every method takes a context first; cancelling it ends the
// call with a *sagat.Error of code CANCELLED that wraps the context's error.
package client

import (
	"context"
	"crypto/tls"
	"io"
	"log/slog"
	"net"
	"net/http"
	"net/url"
	"regexp"
	"strings"
	"sync"
	"time"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
)

const defaultTimeout = 30 * time.Second

// Client calls the REST API of one namespace with one token source.
type Client struct {
	apiURL       string
	namespace    string
	token        sagat.TokenSource
	http         *http.Client
	retry        sagat.RetryPolicy
	timeout      time.Duration
	userAgent    string
	traceContext func(context.Context) string
	logger       *slog.Logger

	mu    sync.Mutex
	names map[string]nameEntry
}

type nameEntry struct {
	id string
	at time.Time
}

type config struct {
	timeout      time.Duration
	retry        sagat.RetryPolicy
	userAgent    string
	httpClient   *http.Client
	tlsConfig    *tls.Config
	traceContext func(context.Context) string
	logger       *slog.Logger
}

// Option configures a Client.
type Option func(*config)

// WithTimeout sets the deadline of one REST call, each attempt of a retried call included; 30 s by
// default.
func WithTimeout(d time.Duration) Option { return func(c *config) { c.timeout = d } }

// WithRetry sets the retry policy; a zero field keeps its default. MaxAttempts of 1 turns retries
// off.
func WithRetry(p sagat.RetryPolicy) Option { return func(c *config) { c.retry = p } }

// WithUserAgent appends a suffix to the User-Agent, which is sagat-sdk-go/<version>.
func WithUserAgent(suffix string) Option { return func(c *config) { c.userAgent = suffix } }

// WithHTTPClient sets the HTTP client, and so its transport: the hook for proxies, custom roots and
// the fault injection of tests. The SDK sets the deadline of each attempt itself; a Timeout on the
// client applies as well. WithTLSConfig is ignored when this is used. The client is copied and the
// copy never follows a redirect (its CheckRedirect is replaced); a Transport of your own must not
// follow redirects either.
func WithHTTPClient(c *http.Client) Option { return func(cfg *config) { cfg.httpClient = c } }

// WithTLSConfig sets the TLS configuration of the default HTTP client: trust roots, and the client
// certificate of mTLS.
func WithTLSConfig(t *tls.Config) Option { return func(c *config) { c.tlsConfig = t } }

// WithTraceContext sets the ambient trace context: it is called for every call that was not given an
// explicit traceparent and returns a W3C traceparent, or "" for none. With OpenTelemetry installed:
//
//	client.WithTraceContext(func(ctx context.Context) string {
//		sc := trace.SpanContextFromContext(ctx)
//		if !sc.IsValid() { return "" }
//		return fmt.Sprintf("00-%s-%s-%s", sc.TraceID(), sc.SpanID(), sc.TraceFlags())
//	})
func WithTraceContext(f func(ctx context.Context) string) Option {
	return func(c *config) { c.traceContext = f }
}

// WithLogger sets the logger; nothing is logged by default. The SDK never logs a payload, a token
// or a secret.
func WithLogger(l *slog.Logger) Option { return func(c *config) { c.logger = l } }

// New returns a Client for the API at apiURL (including /v1, for example https://api.example/v1) and
// the namespace with the given slug. The tenant is not configured: it is the token's.
func New(apiURL, namespace string, token sagat.TokenSource, opts ...Option) (*Client, error) {
	var cfg config
	for _, opt := range opts {
		opt(&cfg)
	}
	if apiURL == "" || namespace == "" {
		return nil, sagat.Invalid("apiURL and namespace are required")
	}
	if token == nil {
		return nil, sagat.Invalid("a token source is required")
	}
	parsed, err := url.Parse(apiURL)
	if err != nil || (parsed.Scheme != "http" && parsed.Scheme != "https") || parsed.Host == "" {
		return nil, sagat.Invalid("apiURL is not an http:// or https:// URL")
	}
	c := &Client{
		apiURL:       strings.TrimRight(apiURL, "/"),
		namespace:    namespace,
		token:        token,
		retry:        cfg.retry.WithDefaults(),
		timeout:      cfg.timeout,
		userAgent:    "sagat-sdk-go/" + sagat.Version,
		traceContext: cfg.traceContext,
		logger:       cfg.logger,
		names:        map[string]nameEntry{},
	}
	if c.timeout <= 0 {
		c.timeout = defaultTimeout
	}
	if cfg.userAgent != "" {
		c.userAgent += " " + cfg.userAgent
	}
	if c.logger == nil {
		c.logger = slog.New(slog.DiscardHandler)
	}
	// A redirect is never followed (sdks/SPEC.md, section 1): a Location can point anywhere, and the request
	// carries a bearer token, the namespace and an idempotency key. A client the caller supplied is
	// copied, so that theirs is not changed.
	if cfg.httpClient != nil {
		copied := *cfg.httpClient
		c.http = &copied
	} else {
		c.http = &http.Client{Transport: defaultTransport(cfg.tlsConfig)}
	}
	c.http.CheckRedirect = func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }
	if parsed.Scheme == "http" && !isLoopback(parsed.Hostname()) {
		c.logger.Warn("the API is reached without TLS outside loopback", "host", parsed.Hostname())
	}
	return c, nil
}

// defaultTransport is a private copy of http.DefaultTransport (an application may have replaced that
// one by a wrapper that is not an *http.Transport), with the TLS configuration if one was given.
func defaultTransport(tlsConfig *tls.Config) *http.Transport {
	transport, ok := http.DefaultTransport.(*http.Transport)
	if ok {
		transport = transport.Clone()
	} else {
		transport = &http.Transport{
			Proxy:               http.ProxyFromEnvironment,
			ForceAttemptHTTP2:   true,
			MaxIdleConns:        100,
			IdleConnTimeout:     90 * time.Second,
			TLSHandshakeTimeout: 10 * time.Second,
		}
	}
	if tlsConfig != nil {
		transport.TLSClientConfig = tlsConfig.Clone()
	}
	return transport
}

func isLoopback(host string) bool {
	if host == "localhost" {
		return true
	}
	ip := net.ParseIP(host)
	return ip != nil && ip.IsLoopback()
}

// Close releases the idle connections of the default HTTP client. It is optional.
func (c *Client) Close() {
	c.http.CloseIdleConnections()
}

var (
	uuidPattern = regexp.MustCompile(`^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$`)
	keyPattern  = regexp.MustCompile(`^[\x21-\x7e]{8,160}$`)
)

// maxAnswer bounds what the client reads of one answer.
const maxAnswer = 32 << 20

func readAll(body io.Reader) ([]byte, error) {
	data, err := io.ReadAll(io.LimitReader(body, maxAnswer+1))
	if err == nil && len(data) > maxAnswer {
		return nil, io.ErrUnexpectedEOF
	}
	return data, err
}
