package worker

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"io"
	"testing"

	"google.golang.org/grpc/metadata"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	pb "github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/workerpb"
)

type fakeBlobReadStream struct {
	parts []*pb.ReadBlobResponse
	err   error
}

func (f *fakeBlobReadStream) Recv() (*pb.ReadBlobResponse, error) {
	if len(f.parts) == 0 {
		return nil, f.err
	}
	part := f.parts[0]
	f.parts = f.parts[1:]
	return part, nil
}
func (*fakeBlobReadStream) Header() (metadata.MD, error) { return nil, nil }
func (*fakeBlobReadStream) Trailer() metadata.MD         { return nil }
func (*fakeBlobReadStream) CloseSend() error             { return nil }
func (*fakeBlobReadStream) Context() context.Context     { return context.Background() }
func (*fakeBlobReadStream) SendMsg(any) error            { return nil }
func (*fakeBlobReadStream) RecvMsg(any) error            { return nil }

func blobDescriptor(body []byte) BlobDescriptor {
	digest := sha256.Sum256(body)
	return BlobDescriptor{ID: "6f27bc42-c41a-4f77-b7db-e9ad53c0231a", Size: int64(len(body)), SHA256: hex.EncodeToString(digest[:]), ContentType: "application/octet-stream"}
}

func headerOf(d BlobDescriptor) *pb.ReadBlobResponse {
	return &pb.ReadBlobResponse{Part: &pb.ReadBlobResponse_Header{Header: &pb.ReadBlobHeader{BlobId: d.ID, Size: d.Size, Sha256: d.SHA256, ContentType: d.ContentType}}}
}

func chunkOf(body []byte) *pb.ReadBlobResponse {
	return &pb.ReadBlobResponse{Part: &pb.ReadBlobResponse_Chunk{Chunk: body}}
}

func readerOf(d BlobDescriptor, parts ...*pb.ReadBlobResponse) *verifyingBlobReader {
	return &verifyingBlobReader{caller: context.Background(), stream: &fakeBlobReadStream{parts: parts, err: io.EOF}, cancel: func() {}, descriptor: d, digest: sha256.New()}
}

func TestBlobReadOnlyReturnsCleanEOFAfterIntegrityVerification(t *testing.T) {
	body := append(make([]byte, maxBlobChunk), []byte("tail")...)
	d := blobDescriptor(body)
	got, err := io.ReadAll(readerOf(d, headerOf(d), chunkOf(body[:maxBlobChunk]), chunkOf(body[maxBlobChunk:])))
	if err != nil || string(got) != string(body) {
		t.Fatalf("read %d bytes: %v", len(got), err)
	}
}

func TestBlobReadRejectsMismatchedHashAfterTheLastChunk(t *testing.T) {
	body := []byte("body")
	d := blobDescriptor(body)
	d.SHA256 = string([]byte(d.SHA256[:63])) + "0"
	_, err := io.ReadAll(readerOf(d, headerOf(d), chunkOf(body)))
	if !sagat.IsCode(err, sagat.CodeProtocol) {
		t.Fatalf("want PROTOCOL, got %v", err)
	}
}

func TestBlobReadDoesNotTurnAPartialTransportFailureIntoEOF(t *testing.T) {
	body := []byte("body")
	d := blobDescriptor(body)
	stream := &fakeBlobReadStream{parts: []*pb.ReadBlobResponse{headerOf(d), chunkOf(body[:2])}, err: errors.New("lost")}
	reader := &verifyingBlobReader{caller: context.Background(), stream: stream, cancel: func() {}, descriptor: d, digest: sha256.New()}
	_, err := io.ReadAll(reader)
	if err == nil {
		t.Fatal("partial transport failure was accepted")
	}
}

func TestBlobOutputIsAnExplicitWrapper(t *testing.T) {
	d := blobDescriptor(nil)
	wrapped := NewBlobOutput(d)
	if wrapped.descriptor != d {
		t.Fatal("wrapper lost its descriptor")
	}
}
