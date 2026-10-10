// Package worker is the worker host of the Go SDK (sdks/SPEC.md, section 6): it registers the
// activities a process can run, takes leases from the worker service, runs the handlers with a
// heartbeat, and reports their result, with the cancellation, lease loss, compensation,
// reconciliation, secrets and protocol negotiation the worker protocol defines.
//
//	host, err := worker.New("https://workers.example:443", "payments", "payments-worker", sagat.StaticToken(token))
//	host.RegisterActivity("payments.charge", worker.Typed(charge))
//	if err := host.Start(ctx); err != nil { ... }
//	defer host.Stop(shutdownCtx)
//
// The host never runs workflow code and never decides a transition: the server owns both.
package worker

import (
	"context"
	"crypto/tls"
	"errors"
	"log/slog"
	"net"
	"net/url"
	"sort"
	"sync"
	"sync/atomic"
	"time"

	"google.golang.org/grpc"
	"google.golang.org/grpc/backoff"
	"google.golang.org/grpc/credentials"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/keepalive"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	boff "github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/backoff"
	pb "github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/workerpb"
)

const (
	defaultMaxInflight = 10
	maxPoll            = 64
	maxOutputBytes     = 256 * 1024
	reportAttempts     = 5
	defaultGrace       = 30 * time.Second
)

// EventType names a thing that happens in the host; see Event.
type EventType string

// The events a host reports, the hook for metrics.
const (
	EventRegistered EventType = "registered"
	EventPoll       EventType = "poll"
	EventTaskTaken  EventType = "task.taken"
	EventCompleted  EventType = "task.completed"
	EventFailed     EventType = "task.failed"
	EventStale      EventType = "task.stale"
	EventDropped    EventType = "task.dropped"
	EventHeartbeat  EventType = "heartbeat"
	EventReconnect  EventType = "reconnect"
)

// Event is a thing that happened in the host. Only the fields that belong to its Type are set. An
// event never carries an input, an output, a lease token or a secret.
type Event struct {
	Type            EventType
	ProtocolVersion string // EventRegistered
	Tasks           int    // EventPoll: how many tasks the poll brought
	TaskID          string
	Activity        string
	Attempt         int
	Code            string        // EventFailed: the error code reported; EventDropped: why
	Retryable       bool          // EventFailed
	Duration        time.Duration // EventCompleted, EventFailed: the time since the task was taken
}

type config struct {
	maxInflight       int
	languages         []string
	pools             []string
	tlsConfig         *tls.Config
	timeout           time.Duration
	retry             sagat.RetryPolicy
	protocolVersion   string
	heartbeatInterval time.Duration
	logger            *slog.Logger
	onEvent           func(Event)
	dialOptions       []grpc.DialOption
}

// Option configures a Host.
type Option func(*config)

// WithMaxInflight sets how many tasks run at once, 1 to 1000; 10 by default.
func WithMaxInflight(n int) Option { return func(c *config) { c.maxInflight = n } }

// WithLanguages sets the languages capability; "go" by default.
func WithLanguages(languages ...string) Option { return func(c *config) { c.languages = languages } }

// WithPools sets the pools capability.
func WithPools(pools ...string) Option { return func(c *config) { c.pools = pools } }

// WithTLSConfig sets the TLS configuration of an https:// worker service: trust roots, and the client
// certificate of mTLS.
func WithTLSConfig(t *tls.Config) Option { return func(c *config) { c.tlsConfig = t } }

// WithTimeout sets the deadline of one unary RPC; 30 s by default.
func WithTimeout(d time.Duration) Option { return func(c *config) { c.timeout = d } }

// WithRetry sets the retry policy of registration and of the waits of the poll loop; a zero field
// keeps its default.
func WithRetry(p sagat.RetryPolicy) Option { return func(c *config) { c.retry = p } }

// WithProtocolVersion overrides the newest protocol minor the host offers (tests use it to play an
// older or a newer SDK).
func WithProtocolVersion(version string) Option {
	return func(c *config) { c.protocolVersion = version }
}

// WithHeartbeatInterval fixes the time between two heartbeats of a task. By default it is a third of
// the time left on the lease, never under 500 ms and never over 10 s.
func WithHeartbeatInterval(d time.Duration) Option {
	return func(c *config) { c.heartbeatInterval = d }
}

// WithLogger sets the logger; nothing is logged by default. The host never logs a payload, a token, a
// lease token, a session token or a secret.
func WithLogger(l *slog.Logger) Option { return func(c *config) { c.logger = l } }

// WithEventHandler sets a function called for each thing that happens in the host. It is called from
// several goroutines and must be fast and safe for concurrent use.
func WithEventHandler(f func(Event)) Option { return func(c *config) { c.onEvent = f } }

// WithDialOptions adds options to the gRPC channel: the hook for interceptors, proxies and the fault
// injection of tests.
func WithDialOptions(opts ...grpc.DialOption) Option {
	return func(c *config) { c.dialOptions = append(c.dialOptions, opts...) }
}

type registered struct {
	kind       string // "activity", "compensation" or "reconciler"
	handler    Handler
	reconciler ReconcilerHandler
}

type session struct {
	workerID string
	token    string
	accepted string
}

type running struct {
	cancel context.CancelCauseFunc
	done   chan struct{}
}

// Host is a worker host. Register the handlers, then Start it; Stop ends it. A Host can be started
// again after it was stopped.
type Host struct {
	grpcURL   string
	namespace string
	name      string
	token     sagat.TokenSource
	cfg       config
	log       *slog.Logger
	retry     sagat.RetryPolicy
	timeout   time.Duration

	lifecycle sync.Mutex // serializes Start and Stop

	mu       sync.Mutex // guards handlers, active and tasks
	handlers map[string]*registered
	active   bool
	tasks    map[string]*running
	wg       sync.WaitGroup

	stopping atomic.Bool
	session  atomic.Pointer[session]
	current  *link // the link of the running Start; guarded by lifecycle
	wake     chan struct{}
}

// link is what one Start makes: the channel, the context the reports and heartbeats live in, and the
// loop. Tasks hold the link they were taken on, so a task that outlives a Stop never meets the next
// Start's channel.
type link struct {
	rpc        *rpc
	conn       *grpc.ClientConn
	life       context.Context
	endLife    context.CancelFunc
	loopCancel context.CancelFunc
	loopDone   chan struct{}
}

// New returns a worker host for the worker service at grpcURL (https://host:port; http:// only for
// loopback or development) serving the namespace with the given slug. The name identifies the
// session: there is one live session per worker name and identity, and registering again replaces
// it. Nothing is connected until Start.
func New(grpcURL, namespace, workerName string, token sagat.TokenSource, opts ...Option) (*Host, error) {
	cfg := config{maxInflight: defaultMaxInflight, protocolVersion: sagat.ProtocolVersion}
	for _, opt := range opts {
		opt(&cfg)
	}
	if grpcURL == "" || namespace == "" || workerName == "" {
		return nil, sagat.Invalid("grpcURL, namespace and workerName are required")
	}
	if token == nil {
		return nil, sagat.Invalid("a token source is required")
	}
	if cfg.maxInflight < 1 || cfg.maxInflight > 1000 {
		return nil, sagat.Invalid("maxInflight is between 1 and 1000")
	}
	if _, _, err := targetOf(grpcURL); err != nil {
		return nil, err
	}
	if len(cfg.languages) == 0 {
		cfg.languages = []string{sagat.Language}
	}
	if cfg.logger == nil {
		cfg.logger = slog.New(slog.DiscardHandler)
	}
	if cfg.timeout <= 0 {
		cfg.timeout = 30 * time.Second
	}
	return &Host{
		grpcURL:   grpcURL,
		namespace: namespace,
		name:      workerName,
		token:     token,
		cfg:       cfg,
		log:       cfg.logger.With("worker", workerName),
		retry:     cfg.retry.WithDefaults(),
		timeout:   cfg.timeout,
		handlers:  map[string]*registered{},
		tasks:     map[string]*running{},
		wake:      make(chan struct{}, 1),
	}, nil
}

// targetOf turns https://host:port into the target of the channel and whether it is secure.
func targetOf(grpcURL string) (target string, secure bool, err error) {
	parsed, parseErr := url.Parse(grpcURL)
	if parseErr != nil || (parsed.Scheme != "http" && parsed.Scheme != "https") || parsed.Host == "" {
		return "", false, sagat.Invalid("grpcURL is not an http:// or https:// URL")
	}
	return parsed.Host, parsed.Scheme == "https", nil
}

// ProtocolVersion is the version the server agreed to; empty before the first Start.
func (h *Host) ProtocolVersion() string {
	if s := h.session.Load(); s != nil {
		return s.accepted
	}
	return ""
}

func (h *Host) add(kind, name string, handler Handler, reconciler ReconcilerHandler) error {
	if name == "" || (handler == nil && reconciler == nil) {
		return sagat.Invalid("an activity needs a name and a handler")
	}
	h.mu.Lock()
	defer h.mu.Unlock()
	if h.active {
		return sagat.Invalid("activities are registered before the host starts")
	}
	if _, twice := h.handlers[name]; twice {
		return sagat.Invalid("an activity named %s is registered already", name)
	}
	h.handlers[name] = &registered{kind: kind, handler: handler, reconciler: reconciler}
	return nil
}

// RegisterActivity registers the handler of an activity. Use Typed for a handler of a declared
// input type.
func (h *Host) RegisterActivity(name string, handler Handler) error {
	return h.add("activity", name, handler, nil)
}

// RegisterCompensation registers the handler of the activity a node names in CompensateWith. It runs
// as a COMPENSATION task, undoes the effect of TaskFrom(ctx).TargetNode, and must be idempotent and
// tolerate an effect that never happened. Send TargetIdempotencyKey to the provider.
func (h *Host) RegisterCompensation(name string, handler Handler) error {
	return h.add("compensation", name, handler, nil)
}

// RegisterReconciler registers the handler of the activity a node names in ReconcileWith; it answers
// a Verdict.
func (h *Host) RegisterReconciler(name string, handler ReconcilerHandler) error {
	return h.add("reconciler", name, nil, handler)
}

func (h *Host) emit(e Event) {
	if h.cfg.onEvent != nil {
		h.cfg.onEvent(e)
	}
}

// Start connects, registers with the server and begins the loop; it returns once registered. ctx
// bounds the registration only: the host runs until Stop. Registration is retried per the retry
// policy when it is safe; a refusal (NOT_BOUND, UNSUPPORTED_PROTOCOL...) is returned as it is.
func (h *Host) Start(ctx context.Context) error {
	h.lifecycle.Lock()
	defer h.lifecycle.Unlock()
	h.mu.Lock()
	if h.active {
		h.mu.Unlock()
		return nil
	}
	if len(h.handlers) == 0 {
		h.mu.Unlock()
		return sagat.Invalid("register at least one activity before start")
	}
	h.mu.Unlock()

	target, secure, err := targetOf(h.grpcURL)
	if err != nil {
		return err
	}
	var creds credentials.TransportCredentials
	if secure {
		tlsConfig := &tls.Config{MinVersion: tls.VersionTLS12}
		if h.cfg.tlsConfig != nil {
			tlsConfig = h.cfg.tlsConfig.Clone()
		}
		creds = credentials.NewTLS(tlsConfig)
	} else {
		creds = insecure.NewCredentials()
		if host, _, splitErr := net.SplitHostPort(target); splitErr == nil && !isLoopback(host) {
			h.log.Warn("the worker service is reached without TLS outside loopback", "host", host)
		}
	}
	dialOptions := []grpc.DialOption{
		grpc.WithTransportCredentials(creds),
		grpc.WithKeepaliveParams(keepalive.ClientParameters{Time: 2 * time.Minute, Timeout: 20 * time.Second}),
		grpc.WithDefaultCallOptions(grpc.MaxCallRecvMsgSize(16 << 20)),
		// A connection that was lost is made again soon: the loop waits with its own backoff.
		grpc.WithConnectParams(grpc.ConnectParams{
			Backoff:           backoff.Config{BaseDelay: 100 * time.Millisecond, Multiplier: 1.6, Jitter: 0.2, MaxDelay: 5 * time.Second},
			MinConnectTimeout: 5 * time.Second,
		}),
		grpc.WithUserAgent("sagat-sdk-go/" + sagat.Version),
	}
	dialOptions = append(dialOptions, h.cfg.dialOptions...)
	conn, err := grpc.NewClient(target, dialOptions...)
	if err != nil {
		return sagat.NewError(sagat.CodeTransport, "the channel to the worker service could not be made").WithCause(err)
	}
	life, endLife := context.WithCancel(context.Background())
	l := &link{
		rpc:      &rpc{client: pb.NewWorkerServiceClient(conn), token: h.token, timeout: h.timeout},
		conn:     conn,
		life:     life,
		endLife:  endLife,
		loopDone: make(chan struct{}),
	}
	h.stopping.Store(false)
	if err := h.register(ctx, l); err != nil {
		endLife()
		_ = conn.Close()
		return err
	}
	h.mu.Lock()
	h.active = true
	h.mu.Unlock()
	var loopCtx context.Context
	loopCtx, l.loopCancel = context.WithCancel(life)
	h.current = l
	go h.run(loopCtx, l)
	return nil
}

func isLoopback(host string) bool {
	if host == "localhost" {
		return true
	}
	ip := net.ParseIP(host)
	return ip != nil && ip.IsLoopback()
}

// Stop stops polling, cancels nothing, and waits for the tasks in flight until ctx is done (30 s
// when ctx has no deadline); then it cancels their contexts with the cause ErrShutdown, gives the
// handlers two seconds more to report, and closes the channel. A task that did not finish keeps its
// lease until it lapses and the server leases it again. Stop returns ctx's error when it had to
// cancel tasks, nil otherwise. Stopping a host that is not running does nothing.
func (h *Host) Stop(ctx context.Context) error {
	h.lifecycle.Lock()
	defer h.lifecycle.Unlock()
	h.mu.Lock()
	if !h.active {
		h.mu.Unlock()
		return nil
	}
	h.mu.Unlock()

	if _, bounded := ctx.Deadline(); !bounded {
		var cancel context.CancelFunc
		ctx, cancel = context.WithTimeout(ctx, defaultGrace)
		defer cancel()
	}
	h.stopping.Store(true)
	h.signal()
	// The poll in progress ends and no other starts; what it leased is in flight from here on.
	l := h.current
	select {
	case <-l.loopDone:
	case <-ctx.Done():
		l.loopCancel()
		<-l.loopDone
	}

	finished := make(chan struct{})
	go func() {
		h.wg.Wait()
		close(finished)
	}()
	var result error
	select {
	case <-finished:
	case <-ctx.Done():
		result = ctx.Err()
		h.mu.Lock()
		for _, task := range h.tasks {
			task.cancel(ErrShutdown)
		}
		h.mu.Unlock()
		select {
		case <-finished:
		case <-time.After(2 * time.Second):
			h.log.Warn("tasks did not end after the shutdown: their leases will lapse")
		}
	}
	h.mu.Lock()
	h.active = false
	h.mu.Unlock()
	l.endLife()
	_ = l.conn.Close()
	h.current = nil
	return result
}

func (h *Host) signal() {
	select {
	case h.wake <- struct{}{}:
	default:
	}
}

// ------------------------------------------------------------------------------------ registration

func (h *Host) register(ctx context.Context, l *link) error {
	h.mu.Lock()
	activities := make([]string, 0, len(h.handlers))
	for name := range h.handlers {
		activities = append(activities, name)
	}
	h.mu.Unlock()
	sort.Strings(activities)
	request := &pb.RegisterWorkerRequest{
		WorkerName: h.name,
		Namespace:  h.namespace,
		Capabilities: &pb.WorkerCapabilities{
			Languages:       h.cfg.languages,
			Activities:      activities,
			Pools:           h.cfg.pools,
			MaxInflight:     int32(h.cfg.maxInflight),
			ProtocolVersion: h.cfg.protocolVersion,
		},
	}
	refreshed := false
	for n := 0; ; n++ {
		response, err := l.rpc.register(ctx, request)
		if err == nil {
			h.session.Store(&session{workerID: response.GetWorkerId(), token: response.GetSessionToken(), accepted: response.GetAcceptedProtocolVersion()})
			h.log.Info("worker registered", "protocol", response.GetAcceptedProtocolVersion())
			h.emit(Event{Type: EventRegistered, ProtocolVersion: response.GetAcceptedProtocolVersion()})
			return nil
		}
		var se *sagat.Error
		if !errors.As(err, &se) {
			return err
		}
		// A token the provider can refresh gets one more chance (sdks/SPEC.md, section 5).
		if se.Code == sagat.CodeUnauthenticated && !refreshed && sagat.CanRefresh(h.token) {
			refreshed = true
			continue
		}
		if !se.Retryable || n+1 >= h.retry.MaxAttempts {
			return err
		}
		if err := boff.Sleep(ctx, boff.After(h.retry, n, se, nil)); err != nil {
			return sagat.NewError(sagat.CodeCancelled, "cancelled while registering").WithCause(err)
		}
	}
}
