package worker_test

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
	"time"

	pb "github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/workerpb"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/worker"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

func TestStreamDispatchesBeforeEOFAndKeepsTaskAfterPartialFailure(t *testing.T) {
	f := newFakeService(t)
	f.accepted = "1.4"
	started := make(chan struct{})
	release := make(chan struct{})
	f.stream = func(r *pb.PollTasksRequest, s grpc.ServerStreamingServer[pb.PollTasksResponse]) error {
		if len(f.of("StreamTasks")) > 1 {
			<-s.Context().Done()
			return s.Context().Err()
		}
		if r.MaxTasks != 2 {
			t.Errorf("credit %d", r.MaxTasks)
		}
		if err := s.Send(&pb.PollTasksResponse{Tasks: []*pb.TaskLease{lease()}}); err != nil {
			return err
		}
		select {
		case <-started:
		case <-time.After(time.Second):
			t.Error("handler waited for EOF")
		}
		return status.Error(codes.Unavailable, "private remote error")
	}
	h := newHost(t, f, worker.WithMaxInflight(2))
	must(t, h.RegisterActivity("charge", func(context.Context, json.RawMessage) (any, error) { close(started); <-release; return 1, nil }))
	start(t, h)
	until(t, "stream reconnect", func() bool { return len(f.of("StreamTasks")) >= 2 })
	close(release)
	until(t, "partial task completion", func() bool { return len(f.of("CompleteStep")) == 1 })
	if len(f.of("PollTasks")) != 0 {
		t.Fatal("unexpected polling")
	}
}

func TestStreamAutoFallsBackOnlyAfterUnimplemented(t *testing.T) {
	f := newFakeService(t)
	f.accepted = "1.4"
	f.lease(lease())
	h := newHost(t, f)
	must(t, h.RegisterActivity("charge", rawOK))
	start(t, h)
	until(t, "fallback completes", func() bool { return len(f.of("CompleteStep")) == 1 })
	if len(f.of("StreamTasks")) != 1 {
		t.Fatal("fallback did not remain disabled for this registration")
	}
}

func TestStreamCancelsIdleReceiveOnStop(t *testing.T) {
	f := newFakeService(t)
	f.accepted = "1.4"
	ended := make(chan struct{})
	f.stream = func(_ *pb.PollTasksRequest, s grpc.ServerStreamingServer[pb.PollTasksResponse]) error {
		<-s.Context().Done()
		close(ended)
		return s.Context().Err()
	}
	h := newHost(t, f)
	must(t, h.RegisterActivity("charge", rawOK))
	start(t, h)
	until(t, "stream open", func() bool { return len(f.of("StreamTasks")) == 1 })
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	must(t, h.Stop(ctx))
	select {
	case <-ended:
	case <-ctx.Done():
		t.Fatal("receive not cancelled")
	}
}

func TestStreamCreditDoesNotRefillWhenAHandlerFinishes(t *testing.T) {
	f := newFakeService(t)
	f.accepted = "1.4"
	f.stream = func(r *pb.PollTasksRequest, s grpc.ServerStreamingServer[pb.PollTasksResponse]) error {
		if len(f.of("StreamTasks")) > 1 {
			<-s.Context().Done()
			return s.Context().Err()
		}
		if r.MaxTasks != 1 {
			t.Errorf("credit %d", r.MaxTasks)
		}
		if err := s.Send(&pb.PollTasksResponse{Tasks: []*pb.TaskLease{lease()}}); err != nil {
			return err
		}
		until(t, "first completion", func() bool { return len(f.of("CompleteStep")) == 1 })
		return s.Send(&pb.PollTasksResponse{Tasks: []*pb.TaskLease{lease(func(l *pb.TaskLease) { l.TaskId = "run-2/charge/1"; l.RunId = "run-2" })}})
	}
	h := newHost(t, f, worker.WithMaxInflight(1))
	must(t, h.RegisterActivity("charge", rawOK))
	start(t, h)
	until(t, "receive after overfill", func() bool { return len(f.of("StreamTasks")) >= 2 })
	if len(f.of("CompleteStep")) != 1 {
		t.Fatal("executed more than total window credit")
	}
}

func TestStreamReceivesSuccessiveFramesBeforeWindowEnds(t *testing.T) {
	f := newFakeService(t)
	f.accepted = "1.4"
	f.stream = func(_ *pb.PollTasksRequest, s grpc.ServerStreamingServer[pb.PollTasksResponse]) error {
		if len(f.of("StreamTasks")) > 1 {
			<-s.Context().Done()
			return s.Context().Err()
		}
		for i := 0; i < 2; i++ {
			l := lease()
			if i == 1 {
				l.TaskId = "run-2/charge/1"
				l.RunId = "run-2"
			}
			if err := s.Send(&pb.PollTasksResponse{Tasks: []*pb.TaskLease{l}}); err != nil {
				return err
			}
			until(t, "frame handler completion", func() bool { return len(f.of("CompleteStep")) == i+1 })
		}
		return nil
	}
	h := newHost(t, f, worker.WithMaxInflight(2))
	must(t, h.RegisterActivity("charge", rawOK))
	start(t, h)
	until(t, "both streamed tasks", func() bool { return len(f.of("CompleteStep")) == 2 })
}

func TestStreamSelectionPreservesOlderServersAndExplicitPolling(t *testing.T) {
	for _, mode := range []worker.Transport{worker.TransportAuto, worker.TransportPoll, worker.TransportStream} {
		t.Run(string(mode), func(t *testing.T) {
			f := newFakeService(t)
			f.accepted = "1.3"
			f.lease(lease())
			h := newHost(t, f, worker.WithTransport(mode))
			must(t, h.RegisterActivity("charge", rawOK))
			if mode == worker.TransportStream {
				if err := h.Start(context.Background()); err == nil {
					t.Fatal("explicit streaming accepted old server")
				}
			} else {
				start(t, h)
				until(t, "old-server completion", func() bool { return len(f.of("CompleteStep")) == 1 })
			}
			if len(f.of("StreamTasks")) != 0 {
				t.Fatal("stream attempted on old minor")
			}
		})
	}
	f := newFakeService(t)
	f.accepted = "1.4"
	f.lease(lease())
	h := newHost(t, f, worker.WithTransport(worker.TransportPoll))
	must(t, h.RegisterActivity("charge", rawOK))
	start(t, h)
	until(t, "explicit poll completion", func() bool { return len(f.of("CompleteStep")) == 1 })
	if len(f.of("StreamTasks")) != 0 {
		t.Fatal("explicit poll used stream")
	}
}

func TestStreamAuthorizationRefusalDoesNotDowngrade(t *testing.T) {
	f := newFakeService(t)
	f.accepted = "1.4"
	f.always["StreamTasks"] = failure{code: codes.PermissionDenied}
	h := newHost(t, f)
	must(t, h.RegisterActivity("charge", rawOK))
	start(t, h)
	until(t, "stream retry", func() bool { return len(f.of("StreamTasks")) >= 2 })
	if len(f.of("PollTasks")) != 0 {
		t.Fatal("authorization failure downgraded")
	}
}

func TestStreamInputBudgetIsCumulativeAcrossFrames(t *testing.T) {
	f := newFakeService(t)
	f.accepted = "1.4"
	f.stream = func(_ *pb.PollTasksRequest, s grpc.ServerStreamingServer[pb.PollTasksResponse]) error {
		if len(f.of("StreamTasks")) > 1 {
			<-s.Context().Done()
			return s.Context().Err()
		}
		for i := 0; i < 2; i++ {
			l := lease(func(l *pb.TaskLease) { l.InputJson = `"` + strings.Repeat("x", 5<<20) + `"` })
			if i == 1 {
				l.TaskId = "run-2/charge/1"
				l.RunId = "run-2"
			}
			if err := s.Send(&pb.PollTasksResponse{Tasks: []*pb.TaskLease{l}}); err != nil {
				return err
			}
			if i == 0 {
				until(t, "first large task completion", func() bool { return len(f.of("CompleteStep")) == 1 })
			}
		}
		return nil
	}
	h := newHost(t, f, worker.WithMaxInflight(2))
	must(t, h.RegisterActivity("charge", rawOK))
	start(t, h)
	until(t, "reconnect after byte overflow", func() bool { return len(f.of("StreamTasks")) >= 2 })
	if len(f.of("CompleteStep")) != 1 {
		t.Fatal("executed task beyond cumulative input budget")
	}
}

func TestStreamExplicitModeRefusesOlderMinorAfterReregistration(t *testing.T) {
	f := newFakeService(t)
	f.accepted = "1.4"
	f.stream = func(_ *pb.PollTasksRequest, _ grpc.ServerStreamingServer[pb.PollTasksResponse]) error {
		f.mu.Lock()
		f.accepted = "1.3"
		f.mu.Unlock()
		return status.Error(codes.Unauthenticated, "session lost")
	}
	events := make(chan worker.Event, 100)
	h := newHost(t, f, worker.WithTransport(worker.TransportStream), worker.WithEventHandler(func(e worker.Event) { events <- e }))
	must(t, h.RegisterActivity("charge", rawOK))
	start(t, h)
	until(t, "older registration", func() bool { return h.ProtocolVersion() == "1.3" })
	deadline := time.After(time.Second)
	for {
		select {
		case e := <-events:
			if e.Type == worker.EventType("receive.failed") && e.Code == "UNSUPPORTED_PROTOCOL" {
				if len(f.of("PollTasks")) != 0 || len(f.of("StreamTasks")) != 1 {
					t.Fatal("unsupported transport was attempted")
				}
				return
			}
		case <-deadline:
			t.Fatal("explicit transport refusal did not expose canonical error")
		}
	}
}
