# backoff

## Overview

Internal retry delays, wait capping and cancellation-aware sleeping. [backoff.go:Delay]

## Interface

- Intra-SDK API: Delay, After, Jitter, Sleep, CapServerWait and Retryable. [backoff.go:Delay] [backoff.go:After] [backoff.go:Jitter] [backoff.go:Sleep] [backoff.go:CapServerWait] [backoff.go:Retryable]

## Invariants

- Retry eligibility accepts the canonical retryable transport/unavailable/rate-limited/internal failures and privately marked token failures. Server waits cap at 60 seconds. [backoff.go:Retryable] [backoff.go:CapServerWait] [../bound/bound.go:MaxServerWait]

## Side Effects

- Uses random jitter and timers; Sleep observes context cancellation. [backoff.go:Delay] [backoff.go:Jitter] [backoff.go:Sleep]

## Verification

Run from the repository root:

```sh
cd sdks/go && go test -race ./... && go vet ./...
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
