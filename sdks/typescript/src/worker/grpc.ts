import { readFileSync } from 'node:fs';
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
export interface TaskLease {
  task_id: string; run_id: string; step_id: string; attempt: number; generation: number; lease_token: string;
  expires_at_epoch_ms: number; activity_name: string; input_json: string; input_blob_ref: string;
  traceparent: string; tenant_alias: string; idempotency_key: string; task_kind: string;
  target_node: string; target_idempotency_key: string;
}
export interface PollTasksResponse { tasks: TaskLease[]; retry_after_ms: number }
export interface HeartbeatRequest {
  task_id: string; worker_id: string; generation: number; lease_token: string; run_id: string; step_id: string; attempt: number;
}
export interface HeartbeatResponse { lease_expires_at_epoch_ms: number; cancel_requested: boolean }
export interface CompleteStepRequest {
  task_id: string; run_id: string; step_id: string; attempt: number; generation: number; lease_token: string;
  idempotency_key: string; output_json: string; output_blob_ref: string; output_sha256: string; traceparent: string;
}
export interface FailStepRequest {
  task_id: string; run_id: string; step_id: string; attempt: number; generation: number; lease_token: string;
  idempotency_key: string; error_code: string; safe_message: string; retryable: boolean; traceparent: string;
}
export interface ResolveSecretsRequest {
  task_id: string; run_id: string; step_id: string; attempt: number; generation: number; lease_token: string; traceparent: string;
}
export interface ResolvedSecret { name: string; value: Buffer; version: number }
export interface ResolveSecretsResponse { secrets: ResolvedSecret[] }
export interface StepCommandResponse { command_id: string; already_applied: boolean; step_status: string; run_status: string }

export type Method = 'RegisterWorker' | 'PollTasks' | 'Heartbeat' | 'CompleteStep' | 'FailStep' | 'ResolveSecrets';

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
    default:
      return safe('INTERNAL', 'the worker service failed');
  }
}

/** The typed, authenticated, error-mapped worker RPCs. */
export class WorkerRpc {
  private readonly client: WorkerServiceClient;

  constructor(private readonly config: GrpcConfig) {
    const url = new URL(config.grpcUrl);
    const secure = url.protocol === 'https:';
    const credentials = secure
      ? grpc.credentials.createSsl(
          config.tls?.ca ? Buffer.from(config.tls.ca) : undefined,
          config.tls?.key ? Buffer.from(config.tls.key) : undefined,
          config.tls?.cert ? Buffer.from(config.tls.cert) : undefined,
        )
      : grpc.credentials.createInsecure();
    const Service = loadService();
    this.client = new Service(url.host, credentials, {
      interceptors: config.interceptors ?? [],
      'grpc.keepalive_time_ms': 120_000,
      'grpc.max_receive_message_length': 16 * 1024 * 1024,
    }) as unknown as WorkerServiceClient;
  }

  close(): void {
    this.client.close();
  }

  private async call<Req, Res>(method: Method, request: Req, traceparent?: string): Promise<Res> {
    const metadata = new grpc.Metadata();
    metadata.set('authorization', `Bearer ${await resolveToken(this.config.token)}`);
    if (traceparent) metadata.set('traceparent', checkTraceparent(traceparent));
    const deadline = new Date(Date.now() + (this.config.timeoutMs ?? 30_000));
    const fn = this.client[method.charAt(0).toLowerCase() + method.slice(1)] as (
      request: Req,
      metadata: grpc.Metadata,
      options: grpc.CallOptions,
      callback: (error: grpc.ServiceError | null, response?: Res) => void,
    ) => void;
    return new Promise<Res>((resolve, reject) => {
      fn.call(this.client, request, metadata, { deadline }, (error, response) => {
        if (error) reject(errorOfStatus(method, error));
        else resolve(response as Res);
      });
    });
  }

  registerWorker(request: RegisterWorkerRequest): Promise<RegisterWorkerResponse> {
    return this.call('RegisterWorker', request);
  }
  pollTasks(request: PollTasksRequest): Promise<PollTasksResponse> {
    return this.call('PollTasks', request);
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
}
