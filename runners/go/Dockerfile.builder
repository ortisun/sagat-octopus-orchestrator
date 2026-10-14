ARG TARGETARCH
FROM --platform=linux/${TARGETARCH} golang:1.25.3-bookworm@sha256:4f43b271f9673eb7bd0cb3a49cc17b08d8d6ee110277e26dbacc93c43a5a7793 AS cache
ARG TARGETARCH
ENV CGO_ENABLED=0 GOOS=linux GOARCH=${TARGETARCH} GOAMD64=v1 GOTOOLCHAIN=local GOPROXY=off GOSUMDB=off GOWORK=off
COPY runners/go/buildhost.go /src/buildhost.go
COPY runners/go/cacheprog.go /src/cacheprog.go
RUN mkdir -p /opt/sagat/go-cache /out \
    && GOCACHE=/opt/sagat/go-cache go install -trimpath -buildvcs=false -ldflags='-buildid= -s -w' std \
    && go build -trimpath -buildvcs=false -ldflags='-buildid= -s -w' -o /out/buildhost /src/buildhost.go \
    && go build -trimpath -buildvcs=false -ldflags='-buildid= -s -w' -o /out/cacheprog /src/cacheprog.go

FROM --platform=linux/${TARGETARCH} golang:1.25.3-bookworm@sha256:4f43b271f9673eb7bd0cb3a49cc17b08d8d6ee110277e26dbacc93c43a5a7793
COPY --from=cache /opt/sagat/go-cache /opt/sagat/go-cache
COPY --from=cache /out/buildhost /opt/sagat/buildhost
COPY --from=cache /out/cacheprog /opt/sagat/cacheprog
COPY runners/go/octopus.go /opt/sagat/trusted/octopus.go
USER 65534:65534
ENTRYPOINT ["/opt/sagat/buildhost"]
