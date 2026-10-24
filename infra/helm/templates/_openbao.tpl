{{- define "sagat.openbao.validate" -}}
{{- if .Values.openbao.enabled -}}
{{- if not (semverCompare ">=1.29.0-0" .Capabilities.KubeVersion.Version) -}}
{{- fail "openbao requires Kubernetes 1.29 or newer for native renewing sidecars" -}}
{{- end -}}
{{- $b := .Values.openbao -}}
{{- if not (regexMatch "^https://[A-Za-z0-9.-]+(:[0-9]+)?/?$" $b.address) -}}
{{- fail "openbao.address must be an HTTPS origin without credentials, path or query" -}}
{{- end -}}
{{- range $key := list "mount" "authMount" "audience" -}}
{{- if not (regexMatch "^[A-Za-z0-9_-]+$" (get $b $key)) -}}
{{- fail (printf "openbao.%s must be a nonempty single name" $key) -}}
{{- end -}}
{{- end -}}
{{- if or (not $b.caSecret) (not $b.intentKeySecret) -}}
{{- fail "openbao.caSecret and openbao.intentKeySecret are required" -}}
{{- end -}}
{{- if not (regexMatch "@sha256:[a-f0-9]{64}$" $b.agentImage) -}}
{{- fail "openbao.agentImage must be pinned by sha256 digest" -}}
{{- end -}}
{{- range $component := list "control-plane" "execution-engine" "secret-cleanup" -}}
{{- if not (regexMatch "^[A-Za-z0-9_-]+$" (get $b.roles $component)) -}}
{{- fail (printf "openbao.roles.%s is required" $component) -}}
{{- end -}}
{{- end -}}
{{- if or (lt (int $b.cleanup.batch) 1) (gt (int $b.cleanup.batch) 128) (ne (toString $b.cleanup.batch) (toString (int $b.cleanup.batch))) -}}
{{- fail "openbao.cleanup.batch must be a whole number from 1 to 128" -}}
{{- end -}}
{{- if .Values.networkPolicy.enabled -}}
{{- include "sagat.networkPolicy.validatePort" (dict "name" "openbao.network.port" "port" $b.network.port) -}}
{{- include "sagat.networkPolicy.validatePeers" (dict "name" "openbao.network.to" "peers" $b.network.to) -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{- define "sagat.openbao.serviceAccount" -}}
{{- include "sagat.fullname" .root -}}{{- if .root.Values.openbao.enabled -}}-{{ .component }}{{- end -}}
{{- end -}}

{{- define "sagat.openbao.env" -}}
{{- if .root.Values.openbao.enabled }}
- {name: SAGAT_OPENBAO_ADDRESS, value: {{ .root.Values.openbao.address | quote }}}
- {name: SAGAT_OPENBAO_MOUNT, value: {{ .root.Values.openbao.mount | quote }}}
- {name: SAGAT_OPENBAO_TOKEN_FILE, value: /run/openbao/token}
- {name: SAGAT_OPENBAO_CA_FILE, value: /etc/openbao/ca/ca.crt}
{{- if eq .component "control-plane" }}
- {name: SAGAT_OPENBAO_INTENT_KEY_FILE, value: /run/openbao-intent/intent-key}
{{- end }}
{{- end }}
{{- end -}}

{{- define "sagat.openbao.mounts" -}}
{{- if .root.Values.openbao.enabled }}
- {name: openbao-token, mountPath: /run/openbao, readOnly: true}
- {name: openbao-ca, mountPath: /etc/openbao/ca, readOnly: true}
{{- if eq .component "control-plane" }}
- {name: openbao-intent, mountPath: /run/openbao-intent, readOnly: true}
{{- end }}
{{- end }}
{{- end -}}

{{- define "sagat.openbao.volumes" -}}
{{- if .root.Values.openbao.enabled }}
- name: openbao-token
  emptyDir: {medium: Memory, sizeLimit: 1Mi}
- name: openbao-config
  configMap:
    name: {{ include "sagat.fullname" .root }}-openbao-agent
    items: [{key: {{ .component }}.hcl, path: agent.hcl}]
- name: openbao-ca
  secret:
    secretName: {{ .root.Values.openbao.caSecret | quote }}
    defaultMode: 288
    items: [{key: ca.crt, path: ca.crt}]
- name: openbao-identity
  projected:
    defaultMode: 288
    sources:
      - serviceAccountToken: {path: token, audience: {{ .root.Values.openbao.audience | quote }}, expirationSeconds: 600}
{{- if eq .component "control-plane" }}
- name: openbao-intent
  secret:
    secretName: {{ .root.Values.openbao.intentKeySecret | quote }}
    defaultMode: 288
    items: [{key: intent-key, path: intent-key}]
{{- end }}
{{- end }}
{{- end -}}

{{- define "sagat.openbao.agent" -}}
- name: openbao-agent
  {{- if ne .component "secret-cleanup" }}
  restartPolicy: Always
  startupProbe:
    exec: {command: ["/bin/sh", "-ec", "test -s /run/openbao/token"]}
    periodSeconds: 2
    failureThreshold: 60
  {{- end }}
  image: {{ .root.Values.openbao.agentImage | quote }}
  command: ["bao", "agent", "-config=/etc/openbao/config/agent.hcl"]
  resources:
    {{- toYaml .root.Values.openbao.agentResources | nindent 4 }}
  {{- include "sagat.containerSecurity" .root | nindent 2 }}
  volumeMounts:
    - {name: openbao-token, mountPath: /run/openbao}
    - {name: openbao-config, mountPath: /etc/openbao/config, readOnly: true}
    - {name: openbao-ca, mountPath: /etc/openbao/ca, readOnly: true}
    - {name: openbao-identity, mountPath: /run/openbao-identity, readOnly: true}
{{- end -}}
