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
{{- if or (lt (int .Values.engine.shardLeaseSeconds) 3) (gt (int .Values.engine.shardLeaseSeconds) 600) -}}
{{- fail "engine.shardLeaseSeconds must be between 3 and 600" -}}
{{- end -}}
{{- if and .Values.engine.grpc.tlsSecret .Values.engine.grpc.allowPlaintextSecrets -}}
{{- fail "engine.grpc.tlsSecret and engine.grpc.allowPlaintextSecrets contradict each other" -}}
{{- end -}}
{{- $seen := dict -}}
{{- range .Values.workerPools -}}
{{- if hasKey $seen .name -}}
{{- fail (printf "the worker pool %s is named twice" .name) -}}
{{- end -}}
{{- $_ := set $seen .name true -}}
{{- if not (regexMatch "^[a-z0-9._-]{1,63}$" .name) -}}
{{- fail (printf "the worker pool name %q must be 1 to 63 lowercase letters, digits or . _ -" .name) -}}
{{- end -}}
{{- if not .activities -}}
{{- fail (printf "the worker pool %s serves no activity" .name) -}}
{{- end -}}
{{- if not .image -}}
{{- fail (printf "the worker pool %s has no image" .name) -}}
{{- end -}}
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
