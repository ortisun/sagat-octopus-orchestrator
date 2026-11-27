{{/* Names and labels shared by every template. */}}

{{- define "sagat.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 40 | trimSuffix "-" -}}
{{- end -}}

{{- define "sagat.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 40 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s" (include "sagat.name" .) | trunc 40 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{- define "sagat.labels" -}}
app.kubernetes.io/name: {{ include "sagat.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end -}}

{{/* selector labels of a component: control-plane, execution-engine or a pool */}}
{{- define "sagat.selector" -}}
app.kubernetes.io/name: {{ include "sagat.name" .root }}
app.kubernetes.io/instance: {{ .root.Release.Name }}
app.kubernetes.io/component: {{ .component }}
{{- end -}}

{{- define "sagat.image" -}}
{{- printf "%s%s:%s" .root.Values.image.registry .repository (default .root.Chart.AppVersion .root.Values.image.tag) -}}
{{- end -}}

{{/* Fails the render for a value the platform would refuse at start, so that it fails here instead. */}}
{{- define "sagat.validate" -}}
{{- include "sagat.openbao.validate" . -}}
{{- range $switch := list "concurrencyKeys" "rateLimits" -}}
{{- if and (index $.Values.flowControl $switch) (not $.Values.flowControl.digestKeys.existingSecret) -}}
{{- fail (printf "flowControl.%s needs flowControl.digestKeys.existingSecret: a Secret with the key flow-control.keys (D022-057)" $switch) -}}
{{- end -}}
{{- end -}}
{{- if not (has .Values.environment (list "dev" "staging" "prod")) -}}
{{- fail "environment must be dev, staging or prod" -}}
{{- end -}}
{{- if not .Values.database.existingSecret -}}
{{- fail "database.existingSecret is required: a Secret with the key DATABASE_URL (the runtime role's)" -}}
{{- end -}}
{{- range $key := list "issuer" "audience" "jwksUri" -}}
{{- if not (index $.Values.oidc $key) -}}
{{- fail (printf "oidc.%s is required" $key) -}}
{{- end -}}
{{- end -}}
{{- if and .Values.oidc.allowInsecureJwksHttp (ne .Values.environment "dev") -}}
{{- fail "oidc.allowInsecureJwksHttp is accepted only when environment=dev" -}}
{{- end -}}
{{- $cpTls := ne .Values.controlPlane.nativeTls.tlsSecret "" -}}
{{- $cpClientCa := ne .Values.controlPlane.nativeTls.clientCaSecret "" -}}
{{- if ne $cpTls $cpClientCa -}}
{{- fail "controlPlane.nativeTls.tlsSecret and controlPlane.nativeTls.clientCaSecret are required together" -}}
{{- end -}}
{{- if and $cpTls .Values.controlPlane.allowPlaintextSecrets -}}
{{- fail "controlPlane.nativeTls and controlPlane.allowPlaintextSecrets contradict each other" -}}
{{- end -}}
{{- if and $cpTls (not .Values.controlPlane.nativeTls.healthcheckServerName) -}}
{{- fail "controlPlane.nativeTls.healthcheckServerName is required with native TLS" -}}
{{- end -}}
{{- if and (not $cpTls) .Values.controlPlane.nativeTls.healthcheckCaSecret -}}
{{- fail "controlPlane.nativeTls.healthcheckCaSecret needs native TLS" -}}
{{- end -}}
{{- if and .Values.engine.autoscaling.keda.enabled .Values.engine.autoscaling.hpa.enabled -}}
{{- fail "enable at most one of engine.autoscaling.keda and engine.autoscaling.hpa" -}}
{{- end -}}
{{- if and .Values.engine.autoscaling.keda.enabled (gt (int .Values.engine.autoscaling.keda.maxReplicaCount) 64) -}}
{{- fail "engine.autoscaling.keda.maxReplicaCount cannot be more than 64: the scheduler has 64 shards and a replica beyond them holds none" -}}
{{- end -}}
{{- if and .Values.engine.autoscaling.hpa.enabled (gt (int .Values.engine.autoscaling.hpa.maxReplicas) 64) -}}
{{- fail "engine.autoscaling.hpa.maxReplicas cannot be more than 64: the scheduler has 64 shards and a replica beyond them holds none" -}}
{{- end -}}
{{- if gt (int .Values.engine.replicas) 64 -}}
{{- fail "engine.replicas cannot be more than 64: the scheduler has 64 shards and a replica beyond them holds none" -}}
{{- end -}}
{{- if gt (int .Values.database.serverMaxConnections) 0 -}}
{{- $engine := int .Values.engine.replicas -}}
{{- if .Values.engine.autoscaling.keda.enabled -}}{{- $engine = max $engine (int .Values.engine.autoscaling.keda.maxReplicaCount) -}}{{- end -}}
{{- if .Values.engine.autoscaling.hpa.enabled -}}{{- $engine = max $engine (int .Values.engine.autoscaling.hpa.maxReplicas) -}}{{- end -}}
{{- $peak := mul (add (int .Values.controlPlane.replicas) $engine) (int .Values.database.maxConnections) -}}
{{- if and .Values.openbao.enabled .Values.openbao.cleanup.enabled -}}
{{- $peak = add $peak 2 -}}
{{- end -}}
{{- if gt $peak (int .Values.database.serverMaxConnections) -}}
{{- fail (printf "the control plane (%d) and the engine (up to %d) with %d connections each would hold up to %d connections, and the server allows %d (database.serverMaxConnections): lower database.maxConnections or the ceiling of the autoscaler, or put a pooler in front of the server" (int .Values.controlPlane.replicas) $engine (int .Values.database.maxConnections) $peak (int .Values.database.serverMaxConnections)) -}}
{{- end -}}
{{- end -}}
{{- range $name := list "shardLeaseSeconds" "schedulerConcurrency" "schedulerBatch" -}}
{{- $value := index $.Values.engine $name -}}
{{- if ne (toString $value) (toString (int $value)) -}}
{{- fail (printf "engine.%s must be a whole number, not %v" $name $value) -}}
{{- end -}}
{{- end -}}
{{- if or (lt (int .Values.engine.schedulerBatch) 1) (gt (int .Values.engine.schedulerBatch) 5000) -}}
{{- fail "engine.schedulerBatch must be between 1 and 5000" -}}
{{- end -}}
{{- if or (lt (int .Values.engine.schedulerConcurrency) 1) (gt (int .Values.engine.schedulerConcurrency) 64) -}}
{{- fail "engine.schedulerConcurrency must be between 1 and 64" -}}
{{- end -}}
{{- if or (lt (int .Values.engine.shardLeaseSeconds) 3) (gt (int .Values.engine.shardLeaseSeconds) 600) -}}
{{- fail "engine.shardLeaseSeconds must be between 3 and 600" -}}
{{- end -}}
{{- if and (or .Values.secrets.existingSecret .Values.openbao.enabled) (ne .Values.environment "dev") -}}
{{- if not (or $cpTls .Values.controlPlane.allowPlaintextSecrets) -}}
{{- fail "secrets.existingSecret is set, so configure controlPlane.nativeTls or set controlPlane.allowPlaintextSecrets=true to state that TLS is terminated in front of the API" -}}
{{- end -}}
{{- if not (or .Values.engine.grpc.tlsSecret .Values.engine.grpc.allowPlaintextSecrets) -}}
{{- fail "secrets.existingSecret is set, so the engine hands secret values to workers: set engine.grpc.tlsSecret (the engine terminates TLS) or engine.grpc.allowPlaintextSecrets=true if a mesh does (the engine refuses to start otherwise)" -}}
{{- end -}}
{{- end -}}
{{- if and .Values.engine.grpc.tlsSecret .Values.engine.grpc.allowPlaintextSecrets -}}
{{- fail "engine.grpc.tlsSecret and engine.grpc.allowPlaintextSecrets contradict each other" -}}
{{- end -}}
{{- if and .Values.networkPolicy.enabled (not .Values.networkPolicy.metricsFrom) -}}
{{- fail "networkPolicy.metricsFrom cannot be empty: an empty `from` lets everybody reach the unauthenticated metrics and probe ports; list who may scrape, or disable the policy" -}}
{{- end -}}
{{- if gt (len .Values.workerPools) 32 -}}
{{- fail "at most 32 worker pools: each is two series of the engine's metrics (the engine refuses more)" -}}
{{- end -}}
{{- $activities := 0 -}}
{{- $seen := dict -}}
{{- range .Values.workerPools -}}
{{- $activities = add $activities (len .activities) -}}
{{- $pool := .name -}}
{{- if hasKey $seen .name -}}
{{- fail (printf "the worker pool %s is named twice" .name) -}}
{{- end -}}
{{- $_ := set $seen .name true -}}
{{- if not (regexMatch "^[a-z0-9]([-a-z0-9]*[a-z0-9])?$" .name) -}}
{{- fail (printf "the worker pool name %q must be lowercase letters, digits and -, starting and ending with a letter or a digit (it is part of the names of Kubernetes objects)" .name) -}}
{{- end -}}
{{- if gt (add (len (include "sagat.fullname" $)) (len "-worker-") (len .name)) 63 -}}
{{- fail (printf "the worker pool name %q is too long: %s-worker-%s must be at most 63 characters" .name (include "sagat.fullname" $) .name) -}}
{{- end -}}
{{- range .activities -}}
{{- if not (regexMatch "^[!-+.-:<>-~]{1,128}$" .) -}}
{{- fail (printf "an activity of the worker pool %s is empty, longer than 128, or has a space or one of , ; = (printable ASCII without those separators)" $pool) -}}
{{- end -}}
{{- end -}}
{{- if not .activities -}}
{{- fail (printf "the worker pool %s serves no activity" .name) -}}
{{- end -}}
{{- if not .image -}}
{{- fail (printf "the worker pool %s has no image" .name) -}}
{{- end -}}
{{- end -}}
{{- if gt $activities 512 -}}
{{- fail "at most 512 activities in all the worker pools (the engine refuses more)" -}}
{{- end -}}
{{- end -}}

{{/* SAGAT_WORKER_POOLS: name=activity,activity;name=activity */}}
{{- define "sagat.workerPools" -}}
{{- $parts := list -}}
{{- range .Values.workerPools -}}
{{- $parts = append $parts (printf "%s=%s" .name (join "," .activities)) -}}
{{- end -}}
{{- join ";" $parts -}}
{{- end -}}

{{- define "sagat.topologySpread" -}}
{{- if .root.Values.topologySpread.enabled }}
topologySpreadConstraints:
  - maxSkew: {{ .root.Values.topologySpread.zoneMaxSkew }}
    topologyKey: topology.kubernetes.io/zone
    whenUnsatisfiable: ScheduleAnyway
    labelSelector:
      matchLabels:
        {{- include "sagat.selector" . | nindent 8 }}
  - maxSkew: {{ .root.Values.topologySpread.nodeMaxSkew }}
    topologyKey: kubernetes.io/hostname
    whenUnsatisfiable: ScheduleAnyway
    labelSelector:
      matchLabels:
        {{- include "sagat.selector" . | nindent 8 }}
{{- end }}
{{- end -}}

{{/* What every container of the platform runs with. */}}
{{- define "sagat.containerSecurity" -}}
securityContext:
  allowPrivilegeEscalation: false
  readOnlyRootFilesystem: true
  runAsNonRoot: true
  runAsUser: 10001
  capabilities:
    drop: [ALL]
  seccompProfile:
    type: RuntimeDefault
{{- end -}}

{{- define "sagat.oidcEnv" -}}
- {name: SAGAT_ENV, value: {{ .Values.environment | quote }}}
- {name: SAGAT_LOG_FORMAT, value: json}
- {name: SAGAT_OIDC_ISSUER, value: {{ .Values.oidc.issuer | quote }}}
- {name: SAGAT_OIDC_AUDIENCE, value: {{ .Values.oidc.audience | quote }}}
- {name: SAGAT_OIDC_JWKS_URI, value: {{ .Values.oidc.jwksUri | quote }}}
- {name: SAGAT_OIDC_TENANT_CLAIM, value: {{ .Values.oidc.tenantClaim | quote }}}
{{- if .Values.oidc.allowInsecureJwksHttp }}
- {name: SAGAT_ALLOW_INSECURE_JWKS_HTTP, value: "true"}
{{- end }}
- name: DATABASE_URL
  valueFrom:
    secretKeyRef: {name: {{ .Values.database.existingSecret | quote }}, key: DATABASE_URL}
- {name: SAGAT_DB_MAX_CONNECTIONS, value: {{ .Values.database.maxConnections | quote }}}
- {name: SAGAT_DB_STATEMENT_TIMEOUT_MS, value: {{ .Values.database.statementTimeoutMs | quote }}}
{{- if .Values.secrets.existingSecret }}
- name: SAGAT_SECRETS_KEYS
  valueFrom:
    secretKeyRef: {name: {{ .Values.secrets.existingSecret | quote }}, key: SAGAT_SECRETS_KEYS}
{{- end }}
{{- end -}}

{{/* The matchers of a query: pool="x" and the install's own, in braces. */}}
{{- define "sagat.matchers" -}}
{{- $all := list -}}
{{- if .pool }}{{ $all = append $all (printf "pool=%q" .pool) }}{{ end -}}
{{- if .root.Values.metrics.selector }}{{ $all = append $all .root.Values.metrics.selector }}{{ end -}}
{{- if $all }}{{ printf "{%s}" (join "," $all) }}{{ end -}}
{{- end -}}

{{/* Flow control (feature 022, D022-057, D022-118): the two switches and the digest key file, the same for the control plane and the
engine. The keys are a Secret mounted read-only as a file, never an environment value. */}}
{{- define "sagat.flowControl.env" -}}
{{- if .Values.flowControl.concurrencyKeys }}
- {name: SAGAT_FLOW_CONTROL_CONCURRENCY_KEYS_ENABLED, value: "true"}
{{- end }}
{{- if .Values.flowControl.rateLimits }}
- {name: SAGAT_FLOW_CONTROL_RATE_LIMITS_ENABLED, value: "true"}
{{- end }}
{{- if .Values.flowControl.digestKeys.existingSecret }}
- {name: SAGAT_FLOW_CONTROL_DIGEST_KEYS_FILE, value: /etc/sagat/flow-control/flow-control.keys}
{{- end }}
{{- end -}}

{{- define "sagat.flowControl.mounts" -}}
{{- if .Values.flowControl.digestKeys.existingSecret }}
- {name: flow-control-keys, mountPath: /etc/sagat/flow-control, readOnly: true}
{{- end }}
{{- end -}}

{{- define "sagat.flowControl.volumes" -}}
{{- if .Values.flowControl.digestKeys.existingSecret }}
- name: flow-control-keys
  secret:
    secretName: {{ .Values.flowControl.digestKeys.existingSecret | quote }}
    defaultMode: 288
    items: [{key: flow-control.keys, path: flow-control.keys}]
{{- end }}
{{- end -}}

{{/*
The automatic handover's values (ADR-0091 decision 6, D2280): `schedules.autoHandover` is a map (`enabled`, `databaseSecret`,
`retireRole`, ...) or, as the opt-out the ADR names, the boolean `false` (or `true`, with every other value at its default).
*/}}
{{- define "sagat.autoHandover.values" -}}
{{- $given := .Values.schedules.autoHandover -}}
{{- $defaults := dict "enabled" true "databaseSecret" "" "retireRole" false "stable" 3 "horizonSeconds" 600 "waitSeconds" 7200 "minLive" 0 -}}
{{- if kindIs "bool" $given -}}
{{- $_ := set $defaults "enabled" $given -}}
{{- else if kindIs "map" $given -}}
{{- /* key by key: a merge would take `false`, the zero value, for a value not given */ -}}
{{- range $key, $value := $given -}}
{{- $_ := set $defaults $key $value -}}
{{- end -}}
{{- end -}}
{{- toJson $defaults -}}
{{- end -}}

{{- define "sagat.autoHandover.enabled" -}}
{{- $ah := include "sagat.autoHandover.values" . | fromJson -}}
{{- if $ah.enabled -}}true{{- else -}}false{{- end -}}
{{- end -}}

{{/*
`<release>-schedule-trigger-handover-<revision>` cut to 52 characters, then `-` and the first 10 hex digits of the SHA-256 of the whole
name: at most 63, the limit of the `job-name` label (D2268).
*/}}
{{- define "sagat.autoHandover.jobName" -}}
{{- $full := printf "%s-schedule-trigger-handover-%d" .Release.Name (int .Release.Revision) -}}
{{- printf "%s-%s" ($full | trunc 52 | trimSuffix "-") (sha256sum $full | trunc 10) -}}
{{- end -}}
