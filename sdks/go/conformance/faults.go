package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"math"
	"net/http"
	"strconv"
	"strings"
	"sync"

	pb "github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/workerpb"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
)

// fault is a fault of the scenario file (tests/contract/sdk-conformance/README.md, "Faults").
type fault struct {
	Channel      string          `json:"channel"`
	Match        string          `json:"match"`
	Action       string          `json:"action"`
	Times        *int            `json:"times"`
	After        int             `json:"after"`
	Status       int             `json:"status"`
	Code         string          `json:"code"`
	Reason       string          `json:"reason"`
	RetryAfterMs *int64          `json:"retryAfterMs"`
	Body         json.RawMessage `json:"body"`
}

var grpcCodes = map[string]codes.Code{
	"OK": codes.OK, "CANCELLED": codes.Canceled, "UNKNOWN": codes.Unknown, "INVALID_ARGUMENT": codes.InvalidArgument,
	"DEADLINE_EXCEEDED": codes.DeadlineExceeded, "NOT_FOUND": codes.NotFound, "ALREADY_EXISTS": codes.AlreadyExists,
	"PERMISSION_DENIED": codes.PermissionDenied, "RESOURCE_EXHAUSTED": codes.ResourceExhausted,
	"FAILED_PRECONDITION": codes.FailedPrecondition, "ABORTED": codes.Aborted, "OUT_OF_RANGE": codes.OutOfRange,
	"UNIMPLEMENTED": codes.Unimplemented, "INTERNAL": codes.Internal, "UNAVAILABLE": codes.Unavailable,
	"DATA_LOSS": codes.DataLoss, "UNAUTHENTICATED": codes.Unauthenticated,
}

// plan is the faults of a scenario, one plan for every client and worker host of the run. A call is
// counted against every fault that matches it, and the first fault in file order that is due answers
// it; applied() says, per fault and in file order, how many calls it answered.
type plan struct {
	mu       sync.Mutex
	faults   []fault
	seen     []int
	answered []int
}

func newPlan(faults []fault) *plan {
	return &plan{faults: faults, seen: make([]int, len(faults)), answered: make([]int, len(faults))}
}

func (p *plan) applied() []int {
	p.mu.Lock()
	defer p.mu.Unlock()
	return append([]int{}, p.answered...)
}

func (p *plan) next(channel, key string) *fault {
	p.mu.Lock()
	defer p.mu.Unlock()
	due := -1
	for i := range p.faults {
		f := &p.faults[i]
		if f.Channel != channel || !strings.HasPrefix(key, f.Match) {
			continue
		}
		count := p.seen[i]
		p.seen[i] = count + 1
		times := math.MaxInt
		if f.Times != nil {
			times = *f.Times
		}
		if due < 0 && count >= f.After && count-f.After < times {
			due = i
		}
	}
	if due < 0 {
		return nil
	}
	p.answered[due]++
	return &p.faults[due]
}

// transport is the http.RoundTripper of the REST client: what a fault says happens to the call,
// otherwise the real thing.
type transport struct {
	plan    *plan
	inner   http.RoundTripper
	apiPath string
}

func (t *transport) RoundTrip(req *http.Request) (*http.Response, error) {
	path := strings.TrimPrefix(req.URL.Path, t.apiPath)
	if path == "" {
		path = "/"
	}
	f := t.plan.next("http", req.Method+" "+path)
	if f == nil {
		return t.inner.RoundTrip(req)
	}
	switch f.Action {
	case "failBefore":
		closeBody(req)
		return nil, errors.New("connection refused (injected)")
	case "status":
		closeBody(req)
		header := http.Header{"Content-Type": {"application/json"}}
		if f.RetryAfterMs != nil {
			header.Set("Retry-After", strconv.FormatInt((*f.RetryAfterMs+999)/1000, 10))
		}
		body := []byte(f.Body)
		if len(body) == 0 {
			body = []byte("{}")
		}
		code := f.Status
		if code == 0 {
			code = http.StatusServiceUnavailable
		}
		return &http.Response{
			StatusCode: code, Status: strconv.Itoa(code) + " " + http.StatusText(code), Proto: "HTTP/1.1", ProtoMajor: 1, ProtoMinor: 1,
			Header: header, Body: io.NopCloser(bytes.NewReader(body)), ContentLength: int64(len(body)), Request: req,
		}, nil
	default: // dropResponse: the call is made, the server applies it, the answer is lost.
		resp, err := t.inner.RoundTrip(req)
		if err != nil {
			return nil, err
		}
		_, _ = io.Copy(io.Discard, resp.Body)
		_ = resp.Body.Close()
		return nil, errors.New("connection reset (injected)")
	}
}

func closeBody(req *http.Request) {
	if req.Body != nil {
		_ = req.Body.Close()
	}
}

// unary is the interceptor of the worker's gRPC channel.
func (p *plan) unary() grpc.UnaryClientInterceptor {
	return func(ctx context.Context, method string, req, reply any, cc *grpc.ClientConn, invoker grpc.UnaryInvoker, opts ...grpc.CallOption) error {
		name := method[strings.LastIndex(method, "/")+1:]
		f := p.next("grpc", name)
		if f == nil {
			return invoker(ctx, method, req, reply, cc, opts...)
		}
		switch f.Action {
		case "failBefore", "status":
			code := codes.Unavailable
			if f.Action == "status" {
				if c, ok := grpcCodes[f.Code]; ok {
					code = c
				}
			}
			// The call never reaches the server: the failure is answered here, with the trailers a
			// server would have sent.
			trailer := metadata.MD{}
			if f.Reason != "" {
				trailer.Set("x-sagat-error", f.Reason)
			}
			if f.RetryAfterMs != nil {
				trailer.Set("retry-after-ms", strconv.FormatInt(*f.RetryAfterMs, 10))
			}
			for _, opt := range opts {
				if t, ok := opt.(grpc.TrailerCallOption); ok {
					*t.TrailerAddr = trailer
				}
			}
			return status.Error(code, "injected")
		default: // dropResponse: the server applies the call; its answer is replaced by a failure to deliver it.
			if err := invoker(ctx, method, req, reply, cc, opts...); err != nil {
				return err
			}
			return status.Error(codes.Unavailable, "injected")
		}
	}
}

// stream injects receive faults into the actual server-stream transport, preserving prior frames.
func (p *plan) stream() grpc.StreamClientInterceptor {
	return func(ctx context.Context, desc *grpc.StreamDesc, cc *grpc.ClientConn, method string, streamer grpc.Streamer, opts ...grpc.CallOption) (grpc.ClientStream, error) {
		name := method[strings.LastIndex(method, "/")+1:]
		f := p.next("grpc", name)
		if f == nil {
			return streamer(ctx, desc, cc, method, opts...)
		}
		if f.Action == "status" || f.Action == "failBefore" {
			code := codes.Unavailable
			if f.Action == "status" {
				if mapped, ok := grpcCodes[f.Code]; ok {
					code = mapped
				}
			}
			return nil, status.Error(code, "injected")
		}
		call, cancel := context.WithCancel(ctx)
		stream, err := streamer(call, desc, cc, method, opts...)
		if err != nil {
			cancel()
			return nil, err
		}
		return &faultStream{ClientStream: stream, cancel: cancel, action: f.Action}, nil
	}
}

type faultStream struct {
	grpc.ClientStream
	cancel  context.CancelFunc
	action  string
	dropped bool
}

func (s *faultStream) RecvMsg(message any) error {
	if s.dropped {
		return status.Error(codes.Unavailable, "injected")
	}
	err := s.ClientStream.RecvMsg(message)
	if err != nil {
		s.cancel()
		return err
	}
	if frame, ok := message.(interface{ GetTasks() []*pb.TaskLease }); ok && len(frame.GetTasks()) == 0 {
		return nil
	}
	s.dropped = true
	s.cancel()
	if s.action == "dropStreamAfterFirst" {
		return nil
	}
	return status.Error(codes.Unavailable, "injected")
}
