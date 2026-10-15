import { readFileSync } from 'node:fs';
import { isIP } from 'node:net';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import * as grpc from '@grpc/grpc-js';
import * as protoLoader from '@grpc/proto-loader';
import type { TlsOptions, TokenSource } from '../config.js';
import { resolveToken } from '../http.js';
import { SagatError, type ErrorCode } from '../errors.js';
import { checkTraceparent } from '../traceparent.js';

// ---- the messages of worker.proto, as the SDK sees them (int64 fields as numbers, which hold epoch
// milliseconds and generations with room to spare) ---------------------------------------------

export interface WorkerCapabilities {
  languages: string[];
  activities: string[];
  pools: string[];
  max_inflight: number;
  protocol_version: string;
}
export interface RegisterWorkerRequest { worker_name: string; namespace: string; capabilities: WorkerCapabilities }
export interface RegisterWorkerResponse { worker_id: string; session_token: string; accepted_protocol_version: string }
export interface PollTasksRequest { worker_id: string; session_token: string; max_tasks: number }
export interface ScopePathSegment { scope_node_id: string; iteration_index: number }
export interface ExecutionInstanceIdentity { instance_path: ScopePathSegment[]; definition_node_id: string }
export interface TaskLease {
  task_id: string; run_id: string; step_id: string; attempt: number; generation: number; lease_token: string;
  expires_at_epoch_ms: number; activity_name: string; input_json: string; input_blob_ref: string;
  traceparent: string; tenant_alias: string; idempotency_key: string; task_kind: string;
  target_node: string; target_idempotency_key: string;
  instance?: ExecutionInstanceIdentity; target_instance?: ExecutionInstanceIdentity;
}
export interface PollTasksResponse { tasks: TaskLease[]; retry_after_ms: number }
export interface HeartbeatRequest {
  task_id: string; worker_id: string; generation: number; lease_token: string; run_id: string; step_id: string; attempt: number;
  instance?: ExecutionInstanceIdentity;
}
export interface HeartbeatResponse { lease_expires_at_epoch_ms: number; cancel_requested: boolean }
export interface CompleteStepRequest {
  task_id: string; run_id: string; step_id: string; attempt: number; generation: number; lease_token: string;
  idempotency_key: string; output_json: string; output_blob_ref: string; output_sha256: string; traceparent: string;
  instance?: ExecutionInstanceIdentity;
}
export interface FailStepRequest {
  task_id: string; run_id: string; step_id: string; attempt: number; generation: number; lease_token: string;
  idempotency_key: string; error_code: string; safe_message: string; retryable: boolean; traceparent: string;
  instance?: ExecutionInstanceIdentity;
}
export interface ResolveSecretsRequest {
  task_id: string; run_id: string; step_id: string; attempt: number; generation: number; lease_token: string; traceparent: string;
  instance?: ExecutionInstanceIdentity;
}
export interface ResolvedSecret { name: string; value: Buffer; version: number }
export interface ResolveSecretsResponse { secrets: ResolvedSecret[] }
export interface WriteBlobRequest { header?: WriteBlobHeader; chunk?: Buffer }
export interface WriteBlobHeader {
  task_id: string; run_id: string; step_id: string; attempt: number; generation: number; lease_token: string;
  content_type: string; expected_size: number; expected_sha256: string;
  instance?: ExecutionInstanceIdentity;
}
export interface WriteBlobResponse { blob_id: string; size: number; sha256: string; content_type: string }
export interface ReadBlobRequest {
  task_id: string; run_id: string; step_id: string; attempt: number; generation: number; lease_token: string; blob_id: string;
  instance?: ExecutionInstanceIdentity;
}
export interface ReadBlobResponse { header?: WriteBlobResponse; chunk?: Buffer; part?: string }
export interface StepCommandResponse { command_id: string; already_applied: boolean; step_status: string; run_status: string }

export type Method = 'RegisterWorker' | 'PollTasks' | 'StreamTasks' | 'Heartbeat' | 'CompleteStep' | 'FailStep' | 'ResolveSecrets' | 'WriteBlob' | 'ReadBlob';

export interface GrpcConfig {
  grpcUrl: string;
  token: TokenSource;
  tls?: TlsOptions;
  timeoutMs?: number;
  /** Interceptors of the channel: the hook of the conformance suite and of tests. */
  interceptors?: grpc.Interceptor[];
}

const here = dirname(fileURLToPath(import.meta.url));
const PROTO = join(here, '..', '..', '..', 'proto', 'worker.proto');

interface WorkerServiceClient extends grpc.Client {
  [method: string]: unknown;
}

let serviceConstructor: grpc.ServiceClientConstructor | undefined;

function loadService(): grpc.ServiceClientConstructor {
  if (serviceConstructor) return serviceConstructor;
  // The file is read here and not by the loader's own path handling so that a missing file is one clear error.
  readFileSync(PROTO);
  const definition = protoLoader.loadSync(PROTO, {
    keepCase: true,
    longs: Number,
    enums: String,
    defaults: true,
    oneofs: true,
  });
  const loaded = grpc.loadPackageDefinition(definition) as unknown as {
    sagat_octopus_orchestrator: { v1: { WorkerService: grpc.ServiceClientConstructor } };
  };
  serviceConstructor = loaded.sagat_octopus_orchestrator.v1.WorkerService;
  return serviceConstructor;
}

const STATUS_NAMES: Record<number, string> = Object.fromEntries(
  Object.entries(grpc.status).filter(([, v]) => typeof v === 'number').map(([name, value]) => [value as number, name]),
);

/** The mapping of a gRPC failure onto the canonical errors (SPEC §2). */
export function errorOfStatus(method: Method, error: grpc.ServiceError): SagatError {
  const machine = error.metadata?.get('x-sagat-error')?.[0];
  const reason = typeof machine === 'string' ? machine : null;
  const retryAfter = error.metadata?.get('retry-after-ms')?.[0];
  const retryAfterMs = typeof retryAfter === 'string' && /^\d+$/.test(retryAfter) ? Number(retryAfter) : null;
  const statusName = STATUS_NAMES[error.code] ?? 'UNKNOWN';
  const safe = (code: ErrorCode, message: string): SagatError =>
    new SagatError({ code, message, status: statusName, reason, retryAfterMs });
  switch (error.code) {
    case grpc.status.UNAUTHENTICATED:
      return safe('UNAUTHENTICATED', 'the worker service refused the credentials or the session');
    case grpc.status.PERMISSION_DENIED:
      if (reason === 'NOT_BOUND') return safe('NOT_BOUND', 'the identity is not bound to the namespace and activity');
      if (reason === 'SECRET_NOT_USABLE') return safe('SECRET_NOT_USABLE', 'a secret of the step cannot be used');
      return safe('FORBIDDEN', 'the identity may not use the worker service');
    case grpc.status.INVALID_ARGUMENT:
      if (reason === 'UNSUPPORTED_PROTOCOL' || (method === 'RegisterWorker' && /unsupported protocol/i.test(error.details ?? ''))) {
        return safe('UNSUPPORTED_PROTOCOL', 'the server does not speak this protocol version');
      }
      return safe('VALIDATION_FAILED', 'the worker service refused a field');
    case grpc.status.ABORTED:
      if (reason === 'IDEMPOTENCY_KEY_REUSED') return safe('IDEMPOTENCY_KEY_REUSED', 'the idempotency key was used for a different report');
      if (reason === null || reason === 'STALE_ATTEMPT') return safe('STALE_ATTEMPT', 'the lease is no longer current');
      return safe('CONFLICT', 'the request conflicts with the state of the run');
    case grpc.status.FAILED_PRECONDITION:
      return safe('STALE_ATTEMPT', 'the lease is no longer current');
    case grpc.status.NOT_FOUND:
      return safe('NOT_FOUND', 'the worker service does not know it');
    case grpc.status.RESOURCE_EXHAUSTED:
      return safe(reason === 'QUOTA_EXCEEDED' ? 'QUOTA_EXCEEDED' : 'RATE_LIMITED', 'the worker service asks for a slower pace');
    case grpc.status.UNAVAILABLE:
    case grpc.status.DEADLINE_EXCEEDED:
      // The server's own UNAVAILABLE (its storage) is told from the lack of an answer by its trailer.
      if (reason === 'UNAVAILABLE') return safe('UNAVAILABLE', 'the worker service could not serve the call now');
      return new SagatError({ code: 'TRANSPORT', message: 'no answer from the worker service', status: statusName, reason, retryAfterMs });
    case grpc.status.CANCELLED:
      return new SagatError({ code: 'CANCELLED', message: 'the call was cancelled', status: statusName });
    case grpc.status.UNIMPLEMENTED:
      return method === 'StreamTasks'
        ? safe('UNSUPPORTED_PROTOCOL', 'the worker service does not implement task streaming')
        : safe('INTERNAL', 'the worker service failed');
    case grpc.status.DATA_LOSS:
      return safe('PROTOCOL', 'the blob stream failed its integrity check');
    default:
      return safe('INTERNAL', 'the worker service failed');
  }
}

/**
 * What a worker URL must be, checked where the host is made so that a mistake is told then and not at the first
 * poll: `http://` or `https://` (any other scheme would be sent in plaintext by the channel without a word,
 * SPEC §1), and over TLS to an IP address the name of the certificate (a TLS server name cannot be an IP address).
 */
export function checkGrpcConfig(config: GrpcConfig): URL {
  let url: URL;
  try {
    url = new URL(config.grpcUrl);
  } catch {
    throw new SagatError({ code: 'VALIDATION_FAILED', message: 'grpcUrl is not a URL: it is http://host:port or https://host:port' });
  }
  if (url.protocol !== 'https:' && url.protocol !== 'http:') {
    throw new SagatError({ code: 'VALIDATION_FAILED', message: 'grpcUrl is http://host:port or https://host:port' });
  }
  if (url.protocol === 'https:' && isIP(url.hostname.replace(/^\[|\]$/g, '')) !== 0 && !config.tls?.serverName) {
    throw new SagatError({ code: 'VALIDATION_FAILED', message: 'gRPC over TLS to an IP address needs tls.serverName, the name the server certificate was issued for' });
  }
  return url;
}

/** `host:port` for the channel; the port of the scheme when the URL names none (`URL.host` would drop it and grpc-js would dial 443). */
export function grpcTarget(url: URL): string {
  return `${url.hostname}:${url.port || (url.protocol === 'https:' ? '443' : '80')}`;
}

/** The typed, authenticated, error-mapped worker RPCs. */
export class WorkerRpc {
  private readonly client: WorkerServiceClient;

  readonly #config: GrpcConfig;

  constructor(config: GrpcConfig) {
    this.#config = config;
    const url = checkGrpcConfig(config);
    const secure = url.protocol === 'https:';
    const credentials = secure
      ? grpc.credentials.createSsl(
          config.tls?.ca ? Buffer.from(config.tls.ca) : undefined,
          config.tls?.key ? Buffer.from(config.tls.key) : undefined,
          config.tls?.cert ? Buffer.from(config.tls.cert) : undefined,
        )
      : grpc.credentials.createInsecure();
    const Service = loadService();
    this.client = new Service(grpcTarget(url), credentials, {
      interceptors: config.interceptors ?? [],
      ...(secure && config.tls?.serverName ? { 'grpc.ssl_target_name_override': config.tls.serverName } : {}),
      'grpc.keepalive_time_ms': 120_000,
      'grpc.max_receive_message_length': 16 * 1024 * 1024,
    }) as unknown as WorkerServiceClient;
  }

  /** Calls in flight: closing the channel does not end them, so they are cancelled with it. */
  readonly #calls = new Set<grpc.ClientUnaryCall>();
  readonly #polls = new Set<grpc.ClientUnaryCall>();
  readonly #streams = new Set<grpc.ClientReadableStream<PollTasksResponse>>();

  /** Cancels task receives and nothing else: a report already on its way is left to arrive. */
  cancelReceives(): void {
    for (const call of this.#polls) call.cancel();
    for (const call of this.#streams) call.cancel();
  }

  close(): void {
    for (const call of this.#calls) call.cancel();
    this.#calls.clear();
    this.client.close();
  }

  private async call<Req, Res>(method: Method, request: Req, traceparent?: string): Promise<Res> {
    const metadata = new grpc.Metadata();
    metadata.set('authorization', `Bearer ${await resolveToken(this.#config.token)}`);
    if (traceparent) metadata.set('traceparent', checkTraceparent(traceparent));
    const deadline = new Date(Date.now() + (this.#config.timeoutMs ?? 30_000));
    const fn = this.client[method.charAt(0).toLowerCase() + method.slice(1)] as (
      request: Req,
      metadata: grpc.Metadata,
      options: grpc.CallOptions,
      callback: (error: grpc.ServiceError | null, response?: Res) => void,
    ) => grpc.ClientUnaryCall;
    return new Promise<Res>((resolve, reject) => {
      const call = fn.call(this.client, request, metadata, { deadline }, (error, response) => {
        this.#calls.delete(call);
        this.#polls.delete(call);
        if (error) reject(errorOfStatus(method, error));
        else resolve(response as Res);
      });
      this.#calls.add(call);
      if (method === 'PollTasks') this.#polls.add(call);
    });
  }

  registerWorker(request: RegisterWorkerRequest): Promise<RegisterWorkerResponse> {
    return this.call('RegisterWorker', request);
  }
  pollTasks(request: PollTasksRequest): Promise<PollTasksResponse> {
    return this.call('PollTasks', request);
  }
  async *streamTasks(request: PollTasksRequest): AsyncIterable<PollTasksResponse> {
    const metadata = new grpc.Metadata();
    metadata.set('authorization', `Bearer ${await resolveToken(this.#config.token)}`);
    const fn = this.client.streamTasks as (
      request: PollTasksRequest,
      metadata: grpc.Metadata,
      options: grpc.CallOptions,
    ) => grpc.ClientReadableStream<PollTasksResponse>;
    const call = fn.call(this.client, request, metadata, {
      // The server window is five seconds; one extra second bounds trailer delivery without holding stop forever.
      deadline: new Date(Date.now() + Math.min(this.#config.timeoutMs ?? 30_000, 6_000)),
    });
    this.#streams.add(call);
    try {
      for await (const response of call as unknown as AsyncIterable<PollTasksResponse>) yield response;
    } catch (error) {
      if (error && typeof error === 'object' && typeof (error as { code?: unknown }).code === 'number') {
        throw errorOfStatus('StreamTasks', error as grpc.ServiceError);
      }
      throw new SagatError({ code: 'INTERNAL', message: 'the task stream failed' });
    } finally {
      this.#streams.delete(call);
      if (!call.destroyed) call.cancel();
    }
  }
  heartbeat(request: HeartbeatRequest, traceparent?: string): Promise<HeartbeatResponse> {
    return this.call('Heartbeat', request, traceparent);
  }
  completeStep(request: CompleteStepRequest, traceparent?: string): Promise<StepCommandResponse> {
    return this.call('CompleteStep', request, traceparent);
  }
  failStep(request: FailStepRequest, traceparent?: string): Promise<StepCommandResponse> {
    return this.call('FailStep', request, traceparent);
  }
  resolveSecrets(request: ResolveSecretsRequest, traceparent?: string): Promise<ResolveSecretsResponse> {
    return this.call('ResolveSecrets', request, traceparent);
  }

  async writeBlob(parts: AsyncIterable<WriteBlobRequest>, traceparent?: string, signal?: AbortSignal): Promise<WriteBlobResponse> {
    const metadata = new grpc.Metadata();
    metadata.set('authorization', `Bearer ${await resolveToken(this.#config.token)}`);
    if (traceparent) metadata.set('traceparent', checkTraceparent(traceparent));
    const fn = this.client.writeBlob as (metadata: grpc.Metadata, options: grpc.CallOptions,
      callback: (error: grpc.ServiceError | null, response?: WriteBlobResponse) => void) => grpc.ClientWritableStream<WriteBlobRequest>;
    return new Promise<WriteBlobResponse>((resolve, reject) => {
      const call = fn.call(this.client, metadata, { deadline: new Date(Date.now() + (this.#config.timeoutMs ?? 30_000)) }, (error, response) => {
        signal?.removeEventListener('abort', cancel);
        if (error) reject(errorOfStatus('WriteBlob', error)); else resolve(response as WriteBlobResponse);
      });
      const cancel = (): void => call.cancel();
      if (signal?.aborted) cancel(); else signal?.addEventListener('abort', cancel, { once: true });
      void (async () => {
        try {
          for await (const part of parts) {
            if (!call.write(part)) await new Promise<void>((ready) => call.once('drain', ready));
          }
          call.end();
        } catch (error) {
          call.cancel();
          reject(error);
        }
      })();
    });
  }

  async *readBlob(request: ReadBlobRequest, traceparent?: string, signal?: AbortSignal): AsyncIterable<ReadBlobResponse> {
    const metadata = new grpc.Metadata();
    metadata.set('authorization', `Bearer ${await resolveToken(this.#config.token)}`);
    if (traceparent) metadata.set('traceparent', checkTraceparent(traceparent));
    const fn = this.client.readBlob as (request: ReadBlobRequest, metadata: grpc.Metadata, options: grpc.CallOptions) => grpc.ClientReadableStream<ReadBlobResponse>;
    const call = fn.call(this.client, request, metadata, { deadline: new Date(Date.now() + (this.#config.timeoutMs ?? 30_000)) });
    const cancel = (): void => call.cancel();
    if (signal?.aborted) cancel(); else signal?.addEventListener('abort', cancel, { once: true });
    try {
      // Node's Readable async iterator applies transport backpressure. A hand-built `data` queue would
      // allow a slow handler to buffer the whole blob and defeat the streaming contract.
      for await (const value of call as unknown as AsyncIterable<ReadBlobResponse>) {
        yield value;
      }
    } catch (error) {
      if (error && typeof error === 'object' && typeof (error as { code?: unknown }).code === 'number') {
        throw errorOfStatus('ReadBlob', error as grpc.ServiceError);
      }
      throw new SagatError({ code: 'INTERNAL', message: 'the blob stream failed' });
    } finally {
      signal?.removeEventListener('abort', cancel);
      if (!call.destroyed) call.cancel();
    }
  }
}
