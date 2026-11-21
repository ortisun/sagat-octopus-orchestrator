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
	w := newWindow(sess)
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
		tasks := frame.GetTasks()
		if h.frameReceived != nil {
			h.frameReceived(len(tasks))
		}
		if h.stopping.Load() {
			// A host that is stopping starts nothing new: what it read is handed back (from protocol 1.8).
			for _, lease := range tasks {
				h.take(l, w, lease, handBackStopping)
			}
			return 0, sagat.NewError(sagat.CodeCancelled, "task receiving stopped")
		}
		if len(tasks) == 0 {
			return int32(pollWait(frame.GetRetryAfterMs()) / time.Millisecond), nil
		}
		allowed := min(credit-received, h.cfg.maxInflight-h.inflight())
		why, taken := "", 0
		var broken error
		for _, lease := range tasks {
			size := len(lease.GetInputJson())
			switch {
			case why != "":
			case taken >= allowed:
				why, broken = handBackCredit, sagat.NewError(sagat.CodeValidationFailed, "task stream exceeded receive credit")
			case size > (8<<20)-bytes:
				why, broken = handBackMalformed, sagat.NewError(sagat.CodeValidationFailed, "task stream exceeded its input budget")
			}
			if h.take(l, w, lease, why) {
				bytes += size
				received++
				taken++
			}
		}
		h.emit(Event{Type: EventPoll, Tasks: taken})
		if broken != nil {
			return 0, broken
		}
	}
}
