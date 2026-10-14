package dev.sagatoctopusorchestrator.sdk.worker;

import dev.sagatoctopusorchestrator.protocol.v1.Worker;
import dev.sagatoctopusorchestrator.sdk.ErrorCode;
import dev.sagatoctopusorchestrator.sdk.SagatException;
import io.grpc.StatusRuntimeException;
import java.io.IOException;
import java.io.InputStream;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.HexFormat;
import java.util.Iterator;

/** Verifies header, size and SHA-256 before exposing clean EOF. */
final class BlobInputStream extends InputStream {
    private final Iterator<Worker.ReadBlobResponse> parts;
    private final BlobDescriptor descriptor;
    private final CancellationSignal cancellation;
    private final Runnable cancelCall;
    private final MessageDigest digest;
    private byte[] buffer = new byte[0];
    private int offset;
    private long size;
    private boolean headerSeen;
    private boolean finished;
    private boolean closed;

    BlobInputStream(Iterator<Worker.ReadBlobResponse> parts, BlobDescriptor descriptor, CancellationSignal cancellation, Runnable cancelCall) {
        this.parts = parts;
        this.descriptor = descriptor;
        this.cancellation = cancellation;
        this.cancelCall = cancelCall;
        try {
            digest = MessageDigest.getInstance("SHA-256");
        } catch (NoSuchAlgorithmException impossible) {
            throw new IllegalStateException(impossible);
        }
    }

    @Override
    public int read() throws IOException {
        byte[] one = new byte[1];
        return read(one, 0, 1) == -1 ? -1 : one[0] & 0xff;
    }

    @Override
    public int read(byte[] target, int start, int length) throws IOException {
        if (closed) {
            throw new IOException("the blob stream is closed");
        }
        if (length == 0) {
            return 0;
        }
        while (offset == buffer.length) {
            if (finished) {
                return -1;
            }
            if (cancellation.isCancelled()) {
                throw new IOException("the blob stream was cancelled");
            }
            final Worker.ReadBlobResponse part;
            try {
                if (!parts.hasNext()) {
                    verifyEnd();
                    finished = true;
                    cancelCall.run();
                    return -1;
                }
                part = parts.next();
            } catch (StatusRuntimeException error) {
                throw new IOException(GrpcErrors.of("ReadBlob", error));
            }
            if (part.hasHeader()) {
                var header = part.getHeader();
                if (headerSeen || size != 0 || !header.getBlobId().equals(descriptor.id()) ||
                        header.getSize() != descriptor.size() || !header.getSha256().equals(descriptor.sha256()) ||
                        !header.getContentType().equals(descriptor.contentType())) {
                    throw protocol("the blob stream header does not match its descriptor");
                }
                headerSeen = true;
                continue;
            }
            if (!part.hasChunk() || !headerSeen || part.getChunk().size() > 256 * 1024) {
                throw protocol("the blob stream has an invalid part order or chunk size");
            }
            buffer = part.getChunk().toByteArray();
            offset = 0;
            size += buffer.length;
            if (size > descriptor.size()) {
                throw protocol("the blob stream is larger than its descriptor");
            }
            digest.update(buffer);
        }
        int copied = Math.min(length, buffer.length - offset);
        System.arraycopy(buffer, offset, target, start, copied);
        offset += copied;
        return copied;
    }

    private void verifyEnd() throws IOException {
        if (!headerSeen || size != descriptor.size() || !HexFormat.of().formatHex(digest.digest()).equals(descriptor.sha256())) {
            throw protocol("the blob stream ended with mismatched integrity metadata");
        }
    }

    private static IOException protocol(String message) {
        return new IOException(SagatException.of(ErrorCode.PROTOCOL, message));
    }

    @Override
    public void close() {
        closed = true;
        buffer = new byte[0];
        cancelCall.run();
    }
}
