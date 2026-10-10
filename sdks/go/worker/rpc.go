package worker

import (
	"context"
	"errors"
	"strconv"
	"strings"
	"time"

	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/bound"
	pb "github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/workerpb"
)

// The method names are the ones of worker.proto; the fault plan of the conformance suite and the
// error mapping both use them.
const (
	methodRegister       = "RegisterWorker"
	methodPoll           = "PollTasks"
	methodHeartbeat      = "Heartbeat"
	methodCompleteStep   = "CompleteStep"
	methodFailStep       = "FailStep"
	methodResolveSecrets = "ResolveSecrets"
)

var statusNames = map[codes.Code]string{
	codes.OK: "OK", codes.Canceled: "CANCELLED", codes.Unknown: "UNKNOWN", codes.InvalidArgument: "INVALID_ARGUMENT",
	codes.DeadlineExceeded: "DEADLINE_EXCEEDED", codes.NotFound: "NOT_FOUND", codes.AlreadyExists: "ALREADY_EXISTS",
	codes.PermissionDenied: "PERMISSION_DENIED", codes.ResourceExhausted: "RESOURCE_EXHAUSTED",
	codes.FailedPrecondition: "FAILED_PRECONDITION", codes.Aborted: "ABORTED", codes.OutOfRange: "OUT_OF_RANGE",
	codes.Unimplemented: "UNIMPLEMENTED", codes.Internal: "INTERNAL", codes.Unavailable: "UNAVAILABLE",
	codes.DataLoss: "DATA_LOSS", codes.Unauthenticated: "UNAUTHENTICATED",
}

func statusName(code codes.Code) string {
	if name, ok := statusNames[code]; ok {
		return name
	}
	return "UNKNOWN"
}

// rpc is the typed, authenticated, error-mapped client of the worker service.
type rpc struct {
	client  pb.WorkerServiceClient
	token   sagat.TokenSource
	timeout time.Duration
}

// call makes one unary RPC: the token and the traceparent go as metadata, the call has its own
// deadline, and a failure is mapped onto the canonical errors (sdks/SPEC.md, section 2). The
// machine code the server puts in the x-sagat-error trailer is read through grpc.Trailer, so an
// interceptor of the channel sees the very call the SDK made.
func call[Req, Res any](ctx context.Context, r *rpc, method string, fn func(context.Context, *Req, ...grpc.CallOption) (*Res, error), request *Req, traceparent string) (*Res, error) {
	token, err := sagat.BearerToken(ctx, r.token)
	if err != nil {
		return nil, err
	}
	md := metadata.Pairs("authorization", "Bearer "+token)
	// A traceparent the server handed out that is malformed is not sent; it is not the caller's mistake.
	if traceparent != "" && sagat.ValidTraceparent(traceparent) {
		md.Set("traceparent", traceparent)
	}
	callCtx, cancel := context.WithTimeout(metadata.NewOutgoingContext(ctx, md), r.timeout)
	defer cancel()
	var header, trailer metadata.MD
	response, err := fn(callCtx, request, grpc.Header(&header), grpc.Trailer(&trailer))
	if err != nil {
		return nil, errorOfStatus(ctx, method, err, header, trailer)
	}
	return response, nil
}

func first(md metadata.MD, key string) string {
	if values := md.Get(key); len(values) > 0 {
		return values[0]
	}
	return ""
}

// errorOfStatus maps a failed call. The trailer x-sagat-error wins when present (it is also looked
// for in the headers: a server that answers with trailers only sends them there).
func errorOfStatus(caller context.Context, method string, err error, header, trailer metadata.MD) error {
	var own *sagat.Error
	if errors.As(err, &own) { // an interceptor of the channel may return the SDK's own error
		return own
	}
	st, _ := status.FromError(err)
	code := st.Code()
	if caller.Err() != nil && (code == codes.Canceled || code == codes.DeadlineExceeded) {
		return sagat.NewError(sagat.CodeCancelled, "the call was cancelled").WithCause(caller.Err())
	}
	reason := first(trailer, "x-sagat-error")
	if reason == "" {
		reason = first(header, "x-sagat-error")
	}
	reason = bound.Text(reason, bound.MaxReason)
	var retryAfter *time.Duration
	value := first(trailer, "retry-after-ms")
	if value == "" {
		value = first(header, "retry-after-ms")
	}
	// A wait the server asks for is capped (a number too large to read is the longest there is).
	if ms, parseErr := strconv.ParseUint(value, 10, 64); parseErr == nil || errors.Is(parseErr, strconv.ErrRange) {
		d := bound.MaxServerWait
		if parseErr == nil && ms < uint64(bound.MaxServerWait/time.Millisecond) {
			d = time.Duration(ms) * time.Millisecond
		}
		retryAfter = &d
	}
	fail := func(c sagat.Code, message string) *sagat.Error {
		e := sagat.NewError(c, message)
		e.GRPCStatus, e.Reason, e.RetryAfter = statusName(code), reason, retryAfter
		return e.WithCause(err)
	}
	switch code {
	case codes.Unauthenticated:
		return fail(sagat.CodeUnauthenticated, "the worker service refused the credentials or the session")
	case codes.PermissionDenied:
		switch reason {
		case "NOT_BOUND":
			return fail(sagat.CodeNotBound, "the identity is not bound to the namespace and activity")
		case "SECRET_NOT_USABLE":
			return fail(sagat.CodeSecretNotUsable, "a secret of the step cannot be used")
		}
		return fail(sagat.CodeForbidden, "the identity may not use the worker service")
	case codes.InvalidArgument:
		if reason == "UNSUPPORTED_PROTOCOL" || (method == methodRegister && strings.Contains(strings.ToLower(st.Message()), "unsupported protocol")) {
			return fail(sagat.CodeUnsupportedProtocol, "the server does not speak this protocol version")
		}
		return fail(sagat.CodeValidationFailed, "the worker service refused a field")
	case codes.Aborted:
		switch reason {
		case "IDEMPOTENCY_KEY_REUSED":
			return fail(sagat.CodeIdempotencyKeyReused, "the idempotency key was used for a different report")
		case "", "STALE_ATTEMPT":
			return fail(sagat.CodeStaleAttempt, "the lease is no longer current")
		}
		return fail(sagat.CodeConflict, "the request conflicts with the state of the run")
	case codes.FailedPrecondition:
		return fail(sagat.CodeStaleAttempt, "the lease is no longer current")
	case codes.NotFound:
		return fail(sagat.CodeNotFound, "the worker service does not know it")
	case codes.ResourceExhausted:
		if reason == "QUOTA_EXCEEDED" {
			return fail(sagat.CodeQuotaExceeded, "the worker service asks for a slower pace")
		}
		return fail(sagat.CodeRateLimited, "the worker service asks for a slower pace")
	case codes.Unavailable, codes.DeadlineExceeded:
		// The server's own UNAVAILABLE (its storage) is told from the lack of an answer by its trailer.
		if reason == "UNAVAILABLE" {
			return fail(sagat.CodeUnavailable, "the worker service could not serve the call now")
		}
		return fail(sagat.CodeTransport, "no answer from the worker service")
	case codes.Canceled:
		e := sagat.NewError(sagat.CodeCancelled, "the call was cancelled")
		e.GRPCStatus = statusName(code)
		return e.WithCause(err)
	default:
		return fail(sagat.CodeInternal, "the worker service failed")
	}
}

func (r *rpc) register(ctx context.Context, request *pb.RegisterWorkerRequest) (*pb.RegisterWorkerResponse, error) {
	return call(ctx, r, methodRegister, r.client.RegisterWorker, request, "")
}

func (r *rpc) poll(ctx context.Context, request *pb.PollTasksRequest) (*pb.PollTasksResponse, error) {
	return call(ctx, r, methodPoll, r.client.PollTasks, request, "")
}

func (r *rpc) heartbeat(ctx context.Context, request *pb.HeartbeatRequest, traceparent string) (*pb.HeartbeatResponse, error) {
	return call(ctx, r, methodHeartbeat, r.client.Heartbeat, request, traceparent)
}

func (r *rpc) completeStep(ctx context.Context, request *pb.CompleteStepRequest, traceparent string) (*pb.StepCommandResponse, error) {
	return call(ctx, r, methodCompleteStep, r.client.CompleteStep, request, traceparent)
}

func (r *rpc) failStep(ctx context.Context, request *pb.FailStepRequest, traceparent string) (*pb.StepCommandResponse, error) {
	return call(ctx, r, methodFailStep, r.client.FailStep, request, traceparent)
}

func (r *rpc) resolveSecrets(ctx context.Context, request *pb.ResolveSecretsRequest, traceparent string) (*pb.ResolveSecretsResponse, error) {
	return call(ctx, r, methodResolveSecrets, r.client.ResolveSecrets, request, traceparent)
}
