package worker_test

import (
	"context"
	"crypto/tls"
	"fmt"
	"net"
	"sync"
	"testing"
	"time"

	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/credentials"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/peer"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/proto"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	pb "github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/workerpb"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/worker"
)

// call is one RPC as the fake service saw it.
type call struct {
	method  string
	request proto.Message
	md      metadata.MD
	// peer is the common name of the client certificate of the connection, "" when it sent none.
	peer string
	at   time.Time
}

// failure is an answer scripted in place of the success: a gRPC code, with the trailers the real
// service would put.
type failure struct {
	code       codes.Code
	reason     string
	retryAfter string
}

// fakeService is an in-memory worker service the tests script: queued leases, queued failures, a
// record of every call. It speaks real gRPC on a loopback port.
type fakeService struct {
	stream func(*pb.PollTasksRequest, grpc.ServerStreamingServer[pb.PollTasksResponse]) error

	pb.UnimplementedWorkerServiceServer
	t      *testing.T
	server *grpc.Server
	port   int
	scheme string

	mu               sync.Mutex
	calls            []call
	leases           []*pb.TaskLease
	failures         map[string][]failure
	accepted         string
	cancelRequested  bool
	cancelOnce       bool
	heartbeatExpires int64
	secrets          []*pb.ResolvedSecret
	pollDelay        time.Duration
	pollRetryAfterMs int32
	registrations    int
	// always answers every call of a method with the failure, after the queued ones.
	always map[string]failure
	// overfill makes a poll answer with every queued lease, whatever max_tasks asked for: a server that
	// does not keep to the protocol.
	overfill bool
}

func newFakeService(t *testing.T) *fakeService { return newFakeServiceWith(t, nil) }

// newFakeTLSService is a fake service that speaks gRPC over TLS with the given server configuration.
func newFakeTLSService(t *testing.T, serverTLS *tls.Config) *fakeService {
	return newFakeServiceWith(t, serverTLS)
}

func newFakeServiceWith(t *testing.T, serverTLS *tls.Config) *fakeService {
	f := &fakeService{t: t, scheme: "http", failures: map[string][]failure{}, always: map[string]failure{}, accepted: "1.2", pollRetryAfterMs: 50, heartbeatExpires: time.Now().Add(time.Minute).UnixMilli()}
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	f.port = listener.Addr().(*net.TCPAddr).Port
	var options []grpc.ServerOption
	if serverTLS != nil {
		f.scheme = "https"
		options = append(options, grpc.Creds(credentials.NewTLS(serverTLS)))
	}
	f.server = grpc.NewServer(options...)
	pb.RegisterWorkerServiceServer(f.server, f)
	go func() { _ = f.server.Serve(listener) }()
	t.Cleanup(f.server.Stop)
	return f
}

func (f *fakeService) url() string { return fmt.Sprintf("%s://127.0.0.1:%d", f.scheme, f.port) }

// enter records the call and returns the failure scripted for it, if any.
func (f *fakeService) enter(ctx context.Context, method string, request proto.Message) error {
	md, _ := metadata.FromIncomingContext(ctx)
	peerName := ""
	if p, ok := peer.FromContext(ctx); ok {
		if info, ok := p.AuthInfo.(credentials.TLSInfo); ok && len(info.State.PeerCertificates) > 0 {
			peerName = info.State.PeerCertificates[0].Subject.CommonName
		}
	}
	f.mu.Lock()
	defer f.mu.Unlock()
	f.calls = append(f.calls, call{method: method, request: proto.Clone(request), md: md, peer: peerName, at: time.Now()})
	queue := f.failures[method]
	var next failure
	switch always, forever := f.always[method]; {
	case len(queue) > 0:
		next = queue[0]
		f.failures[method] = queue[1:]
	case forever:
		next = always
	default:
		return nil
	}
	trailer := metadata.MD{}
	if next.reason != "" {
		trailer.Set("x-sagat-error", next.reason)
	}
	if next.retryAfter != "" {
		trailer.Set("retry-after-ms", next.retryAfter)
	}
	_ = grpc.SetTrailer(ctx, trailer)
	return status.Error(next.code, "scripted")
}

func (f *fakeService) RegisterWorker(ctx context.Context, r *pb.RegisterWorkerRequest) (*pb.RegisterWorkerResponse, error) {
	if err := f.enter(ctx, "RegisterWorker", r); err != nil {
		return nil, err
	}
	f.mu.Lock()
	defer f.mu.Unlock()
	f.registrations++
	return &pb.RegisterWorkerResponse{WorkerId: "w-1", SessionToken: fmt.Sprintf("session-%d", f.registrations), AcceptedProtocolVersion: f.accepted}, nil
}

func (f *fakeService) PollTasks(ctx context.Context, r *pb.PollTasksRequest) (*pb.PollTasksResponse, error) {
	if err := f.enter(ctx, "PollTasks", r); err != nil {
		return nil, err
	}
	f.mu.Lock()
	delay := f.pollDelay
	f.mu.Unlock()
	if delay > 0 {
		select {
		case <-time.After(delay):
		case <-ctx.Done():
			return nil, ctx.Err()
		}
	}
	f.mu.Lock()
	defer f.mu.Unlock()
	// Like the real service: no more than were asked for.
	n := min(int(r.GetMaxTasks()), len(f.leases))
	if f.overfill {
		n = len(f.leases)
	}
	tasks := f.leases[:n]
	f.leases = f.leases[n:]
	return &pb.PollTasksResponse{Tasks: tasks, RetryAfterMs: f.pollRetryAfterMs}, nil
}

func (f *fakeService) Heartbeat(ctx context.Context, r *pb.HeartbeatRequest) (*pb.HeartbeatResponse, error) {
	if err := f.enter(ctx, "Heartbeat", r); err != nil {
		return nil, err
	}
	f.mu.Lock()
	defer f.mu.Unlock()
	cancel := f.cancelRequested
	if f.cancelOnce {
		f.cancelRequested = false
	}
	return &pb.HeartbeatResponse{LeaseExpiresAtEpochMs: f.heartbeatExpires, CancelRequested: cancel}, nil
}

func (f *fakeService) CompleteStep(ctx context.Context, r *pb.CompleteStepRequest) (*pb.StepCommandResponse, error) {
	if err := f.enter(ctx, "CompleteStep", r); err != nil {
		return nil, err
	}
	return &pb.StepCommandResponse{CommandId: "c", StepStatus: "SUCCEEDED", RunStatus: "RUNNING"}, nil
}

func (f *fakeService) FailStep(ctx context.Context, r *pb.FailStepRequest) (*pb.StepCommandResponse, error) {
	if err := f.enter(ctx, "FailStep", r); err != nil {
		return nil, err
	}
	return &pb.StepCommandResponse{CommandId: "c", StepStatus: "FAILED", RunStatus: "RUNNING"}, nil
}

func (f *fakeService) ResolveSecrets(ctx context.Context, r *pb.ResolveSecretsRequest) (*pb.ResolveSecretsResponse, error) {
	if err := f.enter(ctx, "ResolveSecrets", r); err != nil {
		return nil, err
	}
	f.mu.Lock()
	defer f.mu.Unlock()
	return &pb.ResolveSecretsResponse{Secrets: f.secrets}, nil
}

// of returns the calls of a method, in order.
func (f *fakeService) of(method string) []call {
	f.mu.Lock()
	defer f.mu.Unlock()
	var out []call
	for _, c := range f.calls {
		if c.method == method {
			out = append(out, c)
		}
	}
	return out
}

func (f *fakeService) fail(method string, failures ...failure) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.failures[method] = append(f.failures[method], failures...)
}

func (f *fakeService) lease(leases ...*pb.TaskLease) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.leases = append(f.leases, leases...)
}

func (f *fakeService) set(change func(*fakeService)) {
	f.mu.Lock()
	defer f.mu.Unlock()
	change(f)
}

const traceparent = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01"

func lease(changes ...func(*pb.TaskLease)) *pb.TaskLease {
	l := &pb.TaskLease{
		TaskId: "run-1/charge/1", RunId: "run-1", StepId: "charge", Attempt: 1, Generation: 1, LeaseToken: "lt",
		ExpiresAtEpochMs: time.Now().Add(time.Minute).UnixMilli(), ActivityName: "charge",
		InputJson:   `{"run":{"id":"run-1"},"node":"charge","input":{"amount":5},"steps":{"earlier":{"output":{"ok":true}}}}`,
		Traceparent: traceparent, IdempotencyKey: "run-1:charge",
	}
	for _, change := range changes {
		change(l)
	}
	return l
}

func forActivity(name string, task string) func(*pb.TaskLease) {
	return func(l *pb.TaskLease) {
		l.ActivityName, l.StepId, l.TaskId = name, name, fmt.Sprintf("run-1/%s/%s", name, task)
	}
}

// until waits for a condition, failing the test when it does not come.
func until(t *testing.T, what string, condition func() bool) {
	t.Helper()
	end := time.Now().Add(10 * time.Second)
	for !condition() {
		if time.Now().After(end) {
			t.Fatalf("timed out waiting for %s", what)
		}
		time.Sleep(5 * time.Millisecond)
	}
}

func newHost(t *testing.T, f *fakeService, opts ...worker.Option) *worker.Host {
	t.Helper()
	opts = append([]worker.Option{worker.WithRetry(sagat.RetryPolicy{InitialDelay: 5 * time.Millisecond, MaxDelay: 20 * time.Millisecond})}, opts...)
	h, err := worker.New(f.url(), "payments", "test-worker", sagat.StaticToken("tok"), opts...)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		ctx, cancel := context.WithTimeout(context.Background(), time.Second)
		defer cancel()
		_ = h.Stop(ctx)
	})
	return h
}

func start(t *testing.T, h *worker.Host) {
	t.Helper()
	if err := h.Start(context.Background()); err != nil {
		t.Fatalf("start: %v", err)
	}
}

func must(t *testing.T, err error) {
	t.Helper()
	if err != nil {
		t.Fatal(err)
	}
}

func (f *fakeService) StreamTasks(r *pb.PollTasksRequest, stream grpc.ServerStreamingServer[pb.PollTasksResponse]) error {
	if err := f.enter(stream.Context(), "StreamTasks", r); err != nil {
		return err
	}
	if f.stream == nil {
		return status.Error(codes.Unimplemented, "not supported")
	}
	return f.stream(r, stream)
}
