package worker

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"hash"
	"io"
	"regexp"
	"strings"

	"google.golang.org/grpc"
	"google.golang.org/grpc/metadata"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	pb "github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/workerpb"
)

const maxBlobChunk = 256 << 10

var blobSHA256 = regexp.MustCompile(`^[0-9a-f]{64}$`)

// BlobDescriptor is the verified identity of an immutable blob. Pass the whole descriptor back to
// ReadBlob or NewBlobOutput; do not derive an id-only reference from it.
type BlobDescriptor struct {
	ID          string `json:"id"`
	Size        int64  `json:"size"`
	SHA256      string `json:"sha256"`
	ContentType string `json:"contentType"`
}

// WriteBlobOptions adds optional preconditions to a one-pass upload. Zero values omit them.
type WriteBlobOptions struct {
	ContentType    string
	ExpectedSize   int64
	ExpectedSHA256 string
}

// BlobOutput explicitly selects a blob as an activity result. A JSON object with the same fields
// remains an ordinary inline JSON result.
type BlobOutput struct{ descriptor BlobDescriptor }

// NewBlobOutput wraps a descriptor as the result of an activity.
func NewBlobOutput(descriptor BlobDescriptor) BlobOutput { return BlobOutput{descriptor: descriptor} }

func protocolError(message string) error { return sagat.NewError(sagat.CodeProtocol, message) }

func streamHeader(stream interface{ Header() (metadata.MD, error) }) metadata.MD {
	header, _ := stream.Header()
	return header
}

func validateDescriptor(descriptor BlobDescriptor) error {
	if strings.TrimSpace(descriptor.ID) == "" || descriptor.Size < 0 || !blobSHA256.MatchString(descriptor.SHA256) || strings.TrimSpace(descriptor.ContentType) == "" {
		return protocolError("the blob descriptor is malformed")
	}
	return nil
}

func (h *Host) writeBlob(ctx context.Context, l *link, lease *pb.TaskLease, minor int, source io.Reader, options WriteBlobOptions) (BlobDescriptor, error) {
	if minor < 3 {
		return BlobDescriptor{}, sagat.NewError(sagat.CodeUnsupportedProtocol, "blob streaming requires worker protocol 1.3")
	}
	if options.ExpectedSize < 0 || (options.ExpectedSHA256 != "" && !blobSHA256.MatchString(options.ExpectedSHA256)) {
		return BlobDescriptor{}, sagat.Invalid("blob preconditions are malformed")
	}
	callCtx, cancel, err := l.rpc.streamContext(ctx, lease.GetTraceparent())
	if err != nil {
		return BlobDescriptor{}, err
	}
	defer cancel()
	stream, err := l.rpc.client.WriteBlob(callCtx)
	if err != nil {
		return BlobDescriptor{}, errorOfStatus(ctx, methodWriteBlob, err, nil, nil)
	}
	header := &pb.WriteBlobHeader{TaskId: lease.GetTaskId(), RunId: lease.GetRunId(), StepId: lease.GetStepId(), Attempt: lease.GetAttempt(), Generation: lease.GetGeneration(), LeaseToken: lease.GetLeaseToken(), ContentType: options.ContentType, ExpectedSize: options.ExpectedSize, ExpectedSha256: options.ExpectedSHA256, Instance: instanceOf(lease, minor)}
	if err := stream.Send(&pb.WriteBlobRequest{Part: &pb.WriteBlobRequest_Header{Header: header}}); err != nil {
		return BlobDescriptor{}, errorOfStatus(ctx, methodWriteBlob, err, streamHeader(stream), stream.Trailer())
	}
	digest := sha256.New()
	var size int64
	buffer := make([]byte, maxBlobChunk)
	for {
		n, readErr := source.Read(buffer)
		if n > 0 {
			size += int64(n)
			_, _ = digest.Write(buffer[:n])
			chunk := append([]byte(nil), buffer[:n]...)
			if err := stream.Send(&pb.WriteBlobRequest{Part: &pb.WriteBlobRequest_Chunk{Chunk: chunk}}); err != nil {
				return BlobDescriptor{}, errorOfStatus(ctx, methodWriteBlob, err, streamHeader(stream), stream.Trailer())
			}
		}
		if readErr != nil {
			if !errors.Is(readErr, io.EOF) {
				return BlobDescriptor{}, protocolError("the blob source failed before EOF")
			}
			break
		}
		if n == 0 {
			return BlobDescriptor{}, protocolError("the blob source made no progress")
		}
	}
	response, err := stream.CloseAndRecv()
	if err != nil {
		return BlobDescriptor{}, errorOfStatus(ctx, methodWriteBlob, err, streamHeader(stream), stream.Trailer())
	}
	descriptor := BlobDescriptor{ID: response.GetBlobId(), Size: response.GetSize(), SHA256: response.GetSha256(), ContentType: response.GetContentType()}
	if err := validateDescriptor(descriptor); err != nil {
		return BlobDescriptor{}, err
	}
	if descriptor.Size != size || descriptor.SHA256 != hex.EncodeToString(digest.Sum(nil)) {
		return BlobDescriptor{}, protocolError("the blob service returned a descriptor that does not match the uploaded bytes")
	}
	return descriptor, nil
}

type verifyingBlobReader struct {
	caller     context.Context
	stream     grpc.ServerStreamingClient[pb.ReadBlobResponse]
	cancel     context.CancelFunc
	descriptor BlobDescriptor
	digest     hash.Hash
	size       int64
	buffer     []byte
	headerSeen bool
	finished   bool
}

func (r *verifyingBlobReader) Read(p []byte) (int, error) {
	if len(p) == 0 {
		return 0, nil
	}
	if len(r.buffer) > 0 {
		n := copy(p, r.buffer)
		r.buffer = r.buffer[n:]
		return n, nil
	}
	if r.finished {
		return 0, io.EOF
	}
	for {
		part, err := r.stream.Recv()
		if err != nil {
			if errors.Is(err, io.EOF) {
				if !r.headerSeen || r.size != r.descriptor.Size || hex.EncodeToString(r.digest.Sum(nil)) != r.descriptor.SHA256 {
					return 0, protocolError("the blob stream ended with mismatched integrity metadata")
				}
				r.finished = true
				r.cancel()
				return 0, io.EOF
			}
			return 0, errorOfStatus(r.caller, methodReadBlob, err, streamHeader(r.stream), r.stream.Trailer())
		}
		if header := part.GetHeader(); header != nil {
			if r.headerSeen || r.size != 0 || header.GetBlobId() != r.descriptor.ID || header.GetSize() != r.descriptor.Size || header.GetSha256() != r.descriptor.SHA256 || header.GetContentType() != r.descriptor.ContentType {
				return 0, protocolError("the blob stream header does not match its descriptor")
			}
			r.headerSeen = true
			continue
		}
		chunk := part.GetChunk()
		if !r.headerSeen || len(chunk) > maxBlobChunk {
			return 0, protocolError("the blob stream has an invalid part order or chunk size")
		}
		r.size += int64(len(chunk))
		if r.size > r.descriptor.Size {
			return 0, protocolError("the blob stream is larger than its descriptor")
		}
		_, _ = r.digest.Write(chunk)
		n := copy(p, chunk)
		r.buffer = append(r.buffer[:0], chunk[n:]...)
		if n > 0 {
			return n, nil
		}
	}
}

func (r *verifyingBlobReader) Close() error {
	r.cancel()
	r.finished = true
	r.buffer = nil
	return nil
}

func (h *Host) readBlob(ctx context.Context, l *link, lease *pb.TaskLease, minor int, descriptor BlobDescriptor) (io.ReadCloser, error) {
	if minor < 3 {
		return nil, sagat.NewError(sagat.CodeUnsupportedProtocol, "blob streaming requires worker protocol 1.3")
	}
	if err := validateDescriptor(descriptor); err != nil {
		return nil, err
	}
	callCtx, cancel, err := l.rpc.streamContext(ctx, lease.GetTraceparent())
	if err != nil {
		return nil, err
	}
	request := &pb.ReadBlobRequest{TaskId: lease.GetTaskId(), RunId: lease.GetRunId(), StepId: lease.GetStepId(), Attempt: lease.GetAttempt(), Generation: lease.GetGeneration(), LeaseToken: lease.GetLeaseToken(), BlobId: descriptor.ID, Instance: instanceOf(lease, minor)}
	stream, err := l.rpc.client.ReadBlob(callCtx, request)
	if err != nil {
		cancel()
		return nil, errorOfStatus(ctx, methodReadBlob, err, nil, nil)
	}
	return &verifyingBlobReader{caller: ctx, stream: stream, cancel: cancel, descriptor: descriptor, digest: sha256.New()}, nil
}
