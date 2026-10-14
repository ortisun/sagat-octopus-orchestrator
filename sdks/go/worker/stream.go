package worker

import (
	"context"
	"io"
	"strconv"
	"strings"
	"time"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	pb "github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/workerpb"
)

func supportsStream(version string) bool {
	major, minor, ok := strings.Cut(version, ".")
	if !ok || major != "1" {
		return false
	}
	n, err := strconv.Atoi(minor)
	return err == nil && n >= 4
}

// receive spends finite local credit. Tasks already dispatched outlive failure of this receive.
func (h *Host) receive(ctx context.Context, l *link, sess *session, credit int) (int32, error) {
	callCtx, cancel, err := l.rpc.streamContext(ctx, "")
	if err != nil {
		return 0, err
	}
	defer cancel()
	// The server has a five-second window. Bound receive to six seconds or the shorter configured request timeout.
	callCtx, end := context.WithTimeout(callCtx, 6*time.Second)
	defer end()
	h.mu.Lock()
	if h.stopping.Load() {
		h.mu.Unlock()
		return 0, sagat.NewError(sagat.CodeCancelled, "task receiving stopped")
	}
	h.receiveCancel = end
	h.mu.Unlock()
	defer func() { h.mu.Lock(); h.receiveCancel = nil; h.mu.Unlock() }()
	stream, err := l.rpc.client.StreamTasks(callCtx, &pb.PollTasksRequest{WorkerId: sess.workerID, SessionToken: sess.token, MaxTasks: int32(credit)})
	if err != nil {
		return 0, errorOfStatus(ctx, "StreamTasks", err, nil, nil)
	}
	received, bytes := 0, 0
	for {
		frame, err := stream.Recv()
		if err == io.EOF {
			if received == credit {
				return 0, nil
			}
			return 1000, nil // An underfilled EOF without a terminal hint must not busy-reconnect.
		}
		if err != nil {
			header, _ := stream.Header()
			return 0, errorOfStatus(ctx, "StreamTasks", err, header, stream.Trailer())
		}
		if h.stopping.Load() {
			return 0, sagat.NewError(sagat.CodeCancelled, "task receiving stopped")
		}
		tasks := frame.GetTasks()
		if len(tasks) == 0 {
			return int32(pollWait(frame.GetRetryAfterMs()) / time.Millisecond), nil
		}
		allowed := min(credit-received, h.cfg.maxInflight-h.inflight())
		excess := len(tasks) > allowed
		if excess {
			tasks = tasks[:max(0, allowed)]
		}
		for _, lease := range tasks {
			size := len(lease.GetInputJson())
			if size > (8<<20)-bytes {
				return 0, sagat.NewError(sagat.CodeValidationFailed, "task stream exceeded its input budget")
			}
			bytes += size
			received++
			h.dispatch(l, lease)
		}
		h.emit(Event{Type: EventPoll, Tasks: len(tasks)})
		if excess {
			return 0, sagat.NewError(sagat.CodeValidationFailed, "task stream exceeded receive credit")
		}
	}
}
