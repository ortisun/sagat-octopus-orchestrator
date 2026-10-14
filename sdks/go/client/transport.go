package client

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/url"
	"strconv"
	"time"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/backoff"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/bound"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/jsonx"
)

// restCodes are the codes the REST error envelope may carry; any other is classified by status.
var restCodes = map[sagat.Code]bool{
	sagat.CodeBadRequest: true, sagat.CodeUnauthenticated: true, sagat.CodeForbidden: true, sagat.CodeNotFound: true,
	sagat.CodeConflict: true, sagat.CodeValidationFailed: true, sagat.CodeRateLimited: true,
	sagat.CodeQuotaExceeded: true, sagat.CodeUnavailable: true, sagat.CodeInternal: true,
}

type request struct {
	method string
	// path is under the API URL and starts with /.
	path  string
	query url.Values
	// body is encoded once, before the first attempt, so every attempt sends the same bytes.
	body           any
	hasBody        bool
	rawBody        io.Reader
	contentType    string
	headers        http.Header
	idempotencyKey string
	traceparent    string
	// repeatable is true for a call whose repetition cannot do harm: a GET, or a POST that carries a
	// key or an event id the server deduplicates.
	repeatable bool
}

type response struct {
	status  int
	traceID string
	body    []byte
}

func (c *Client) do(ctx context.Context, req request) (*response, error) {
	var payload []byte
	if req.hasBody {
		var err error
		if payload, err = jsonx.Encode(req.body); err != nil {
			return nil, err
		}
	}
	attempts := 1
	if req.repeatable {
		attempts = c.retry.MaxAttempts
	}
	var last error
	for n := 0; n < attempts; n++ {
		resp, err := c.once(ctx, req, payload)
		if err == nil {
			return resp, nil
		}
		last = err
		var se *sagat.Error
		if !errors.As(err, &se) || !backoff.Retryable(se) || n+1 >= attempts {
			return nil, err
		}
		wait := backoff.After(c.retry, n, se, nil)
		c.logger.Warn("retrying a request", "method", req.method, "path", req.path, "code", string(se.Code), "attempt", n+1, "wait", wait)
		if err := backoff.Sleep(ctx, wait); err != nil {
			return nil, sagat.NewError(sagat.CodeCancelled, "cancelled while waiting to retry").WithCause(err)
		}
	}
	return nil, last
}

func (c *Client) once(ctx context.Context, req request, payload []byte) (*response, error) {
	token, err := sagat.BearerToken(ctx, c.token)
	if err != nil {
		return nil, err
	}
	target := c.apiURL + req.path
	if len(req.query) > 0 {
		target += "?" + req.query.Encode()
	}
	attemptCtx, cancel := context.WithTimeout(ctx, c.timeout)
	defer cancel()
	var reader io.Reader
	if req.hasBody {
		reader = bytes.NewReader(payload)
	} else if req.rawBody != nil {
		reader = req.rawBody
	}
	httpReq, err := http.NewRequestWithContext(attemptCtx, req.method, target, reader)
	if err != nil {
		return nil, sagat.NewError(sagat.CodeBadRequest, "the request URL is not valid")
	}
	httpReq.Header.Set("Authorization", "Bearer "+token)
	httpReq.Header.Set("Accept", "application/json")
	httpReq.Header.Set("X-Octopus-Namespace", c.namespace)
	httpReq.Header.Set("User-Agent", c.userAgent)
	if req.traceparent != "" {
		httpReq.Header.Set("traceparent", req.traceparent)
	}
	if req.idempotencyKey != "" {
		httpReq.Header.Set("Idempotency-Key", req.idempotencyKey)
	}
	if req.hasBody {
		httpReq.Header.Set("Content-Type", "application/json")
	} else if req.rawBody != nil {
		httpReq.Header.Set("Content-Type", req.contentType)
	}
	for name, values := range req.headers {
		for _, value := range values {
			httpReq.Header.Add(name, value)
		}
	}

	resp, err := c.http.Do(httpReq)
	if err != nil {
		return nil, transportError(ctx, attemptCtx, err, "no answer from the server")
	}
	defer resp.Body.Close()
	traceID := bound.TraceID(resp.Header.Get("X-Trace-Id"))
	data, err := readAll(resp.Body)
	if errors.Is(err, errTooLarge) {
		e := sagat.NewError(sagat.CodeProtocol, "the answer is too large")
		e.HTTPStatus, e.TraceID = resp.StatusCode, traceID
		return nil, e
	}
	if err != nil {
		e := transportError(ctx, attemptCtx, err, "the answer was cut off")
		var se *sagat.Error
		if errors.As(e, &se) && se.Code == sagat.CodeTransport {
			se.HTTPStatus, se.TraceID = resp.StatusCode, traceID
		}
		return nil, e
	}
	if resp.StatusCode >= 300 && resp.StatusCode < 400 {
		// Never followed, whatever the target: the message does not say where it pointed.
		e := sagat.NewError(sagat.CodeProtocol, "the server redirected the call")
		e.HTTPStatus, e.TraceID = resp.StatusCode, traceID
		return nil, e
	}
	if resp.StatusCode >= 200 && resp.StatusCode < 300 {
		if len(bytes.TrimSpace(data)) == 0 {
			return &response{status: resp.StatusCode, traceID: traceID}, nil
		}
		if !json.Valid(data) {
			e := sagat.NewError(sagat.CodeProtocol, "the answer is not JSON")
			e.HTTPStatus, e.TraceID = resp.StatusCode, traceID
			return nil, e
		}
		return &response{status: resp.StatusCode, traceID: traceID, body: data}, nil
	}
	return nil, errorOf(resp, data, traceID)
}

// transportError says what went wrong when there was no usable answer: the caller's own
// cancellation, a call that ran out of time, or the network.
func transportError(caller, attempt context.Context, err error, message string) error {
	if caller.Err() != nil {
		return sagat.NewError(sagat.CodeCancelled, "the call was cancelled").WithCause(caller.Err())
	}
	if errors.Is(attempt.Err(), context.DeadlineExceeded) {
		message = "the request timed out"
	}
	return sagat.NewError(sagat.CodeTransport, message).WithCause(err)
}

func errorOf(resp *http.Response, data []byte, traceID string) *sagat.Error {
	status := resp.StatusCode
	var retryAfter *time.Duration
	if header := resp.Header.Get("Retry-After"); header != "" {
		// A wait the server asks for is capped (a number too large to read is the longest there is).
		seconds, err := strconv.ParseUint(header, 10, 64)
		if err == nil || errors.Is(err, strconv.ErrRange) {
			d := bound.MaxServerWait
			if err == nil && seconds < uint64(bound.MaxServerWait/time.Second) {
				d = time.Duration(seconds) * time.Second
			}
			retryAfter = &d
		}
	}
	envelope, envelopeErr := jsonx.Object(data)
	given := sagat.Code("")
	if envelopeErr == nil {
		if code, ok := jsonx.String(envelope, "code"); ok && restCodes[sagat.Code(code)] {
			given = sagat.Code(code)
		}
	}
	code := given
	if code == "" {
		code = sagat.CodeOfHTTPStatus(status)
	}
	message := "the server answered " + strconv.Itoa(status)
	if given != "" {
		if m, ok := jsonx.String(envelope, "message"); ok {
			message = bound.Text(m, bound.MaxMessage)
		}
	}
	e := sagat.NewError(code, message)
	e.HTTPStatus = status
	e.RetryAfter = retryAfter
	e.TraceID = traceID
	if envelopeErr == nil {
		if id, ok := jsonx.String(envelope, "traceId"); ok {
			e.TraceID = bound.TraceID(id)
		}
		if given != "" {
			if r, ok := jsonx.Bool(envelope, "retryable"); ok {
				e.Retryable = r
			}
		}
		if details, err := jsonx.Object(envelope["details"]); err == nil {
			reason, _ := jsonx.String(details, "reason")
			e.Reason = bound.Text(reason, bound.MaxReason)
		}
	}
	// A call the server does not have (an older server, sdks/SPEC.md section 9) is told from a resource that is
	// missing: a 501, a 404 that says ROUTE_NOT_FOUND, a 405 that says METHOD_NOT_ALLOWED, or a 404 that is not
	// the platform's error envelope at all. A 404 for a run that does not exist has the envelope and no such
	// reason, and stays a plain NOT_FOUND.
	if status == http.StatusNotImplemented ||
		(status == http.StatusNotFound && (given == "" || e.Reason == "ROUTE_NOT_FOUND")) ||
		(status == http.StatusMethodNotAllowed && e.Reason == "METHOD_NOT_ALLOWED") {
		e.Code = sagat.CodeNotFound
		e.Retryable = sagat.RetryableByDefault(sagat.CodeNotFound)
		e.Remediation = sagat.Remediation(sagat.CodeNotFound)
		e.Reason = "UNSUPPORTED_BY_SERVER"
	}
	return e
}
