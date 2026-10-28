{{/* Validation used only by the feature 009 egress policies. */}}

{{- define "sagat.networkPolicy.validatePort" -}}
{{- $name := .name -}}
{{- $port := .port -}}
{{- if or (ne (toString $port) (toString (int $port))) (lt (int $port) 1) (gt (int $port) 65535) -}}
{{- fail (printf "%s must be a whole-number port between 1 and 65535" $name) -}}
{{- end -}}
{{- end -}}

{{- define "sagat.networkPolicy.validateSelector" -}}
{{- $name := .name -}}
{{- $selector := .selector -}}
{{- if not (kindIs "map" $selector) -}}
{{- fail (printf "%s must be a Kubernetes label selector" $name) -}}
{{- end -}}
{{- range $key := keys $selector -}}
{{- if not (has $key (list "matchLabels" "matchExpressions")) -}}
{{- fail (printf "%s contains unsupported field %q" $name $key) -}}
{{- end -}}
{{- end -}}
{{- if and (not (get $selector "matchLabels")) (not (get $selector "matchExpressions")) -}}
{{- fail (printf "%s cannot be empty because an empty selector matches globally" $name) -}}
{{- end -}}
{{- end -}}

{{- define "sagat.networkPolicy.validatePeers" -}}
{{- $name := .name -}}
{{- $peers := .peers -}}
{{- if or (not (kindIs "slice" $peers)) (eq (len $peers) 0) -}}
{{- fail (printf "%s must contain at least one explicit peer" $name) -}}
{{- end -}}
{{- range $index, $peer := $peers -}}
{{- $peerName := printf "%s[%d]" $name $index -}}
{{- if or (not (kindIs "map" $peer)) (eq (len $peer) 0) -}}
{{- fail (printf "%s cannot be empty because an empty peer allows every destination" $peerName) -}}
{{- end -}}
{{- range $key := keys $peer -}}
{{- if not (has $key (list "podSelector" "namespaceSelector" "ipBlock")) -}}
{{- fail (printf "%s contains unsupported field %q" $peerName $key) -}}
{{- end -}}
{{- end -}}
{{- if hasKey $peer "ipBlock" -}}
{{- if or (hasKey $peer "podSelector") (hasKey $peer "namespaceSelector") -}}
{{- fail (printf "%s cannot combine ipBlock with podSelector or namespaceSelector" $peerName) -}}
{{- end -}}
{{- $ipBlock := get $peer "ipBlock" -}}
{{- if not (kindIs "map" $ipBlock) -}}
{{- fail (printf "%s.ipBlock must be an object" $peerName) -}}
{{- end -}}
{{- range $key := keys $ipBlock -}}
{{- if not (has $key (list "cidr" "except")) -}}
{{- fail (printf "%s.ipBlock contains unsupported field %q" $peerName $key) -}}
{{- end -}}
{{- end -}}
{{- if not (get $ipBlock "cidr") -}}
{{- fail (printf "%s.ipBlock.cidr is required" $peerName) -}}
{{- end -}}
{{- if regexMatch "/0+$" (toString (get $ipBlock "cidr")) -}}
{{- fail (printf "%s.ipBlock.cidr cannot allow the global address space" $peerName) -}}
{{- end -}}
{{- else -}}
{{- if and (not (hasKey $peer "podSelector")) (not (hasKey $peer "namespaceSelector")) -}}
{{- fail (printf "%s must contain podSelector, namespaceSelector or ipBlock" $peerName) -}}
{{- end -}}
{{- if hasKey $peer "podSelector" -}}
{{- include "sagat.networkPolicy.validateSelector" (dict "name" (printf "%s.podSelector" $peerName) "selector" (get $peer "podSelector")) -}}
{{- end -}}
{{- if hasKey $peer "namespaceSelector" -}}
{{- include "sagat.networkPolicy.validateSelector" (dict "name" (printf "%s.namespaceSelector" $peerName) "selector" (get $peer "namespaceSelector")) -}}
{{- end -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{- define "sagat.networkPolicy.validateRules" -}}
{{- $name := .name -}}
{{- $rules := .rules -}}
{{- $peerKey := .peerKey -}}
{{- if not (kindIs "slice" $rules) -}}
{{- fail (printf "%s must be a list" $name) -}}
{{- end -}}
{{- range $ruleIndex, $rule := $rules -}}
{{- $ruleName := printf "%s[%d]" $name $ruleIndex -}}
{{- if or (not (kindIs "map" $rule)) (eq (len $rule) 0) -}}
{{- fail (printf "%s cannot be empty" $ruleName) -}}
{{- end -}}
{{- range $key := keys $rule -}}
{{- if not (has $key (list $peerKey "ports")) -}}
{{- fail (printf "%s contains unsupported field %q" $ruleName $key) -}}
{{- end -}}
{{- end -}}
{{- include "sagat.networkPolicy.validatePeers" (dict "name" (printf "%s.%s" $ruleName $peerKey) "peers" (get $rule $peerKey)) -}}
{{- $ports := get $rule "ports" -}}
{{- if or (not (kindIs "slice" $ports)) (eq (len $ports) 0) -}}
{{- fail (printf "%s.ports must contain at least one explicit port" $ruleName) -}}
{{- end -}}
{{- range $portIndex, $port := $ports -}}
{{- $portName := printf "%s.ports[%d]" $ruleName $portIndex -}}
{{- if not (kindIs "map" $port) -}}
{{- fail (printf "%s must be an object" $portName) -}}
{{- end -}}
{{- range $key := keys $port -}}
{{- if not (has $key (list "port" "protocol")) -}}
{{- fail (printf "%s contains unsupported field %q" $portName $key) -}}
{{- end -}}
{{- end -}}
{{- include "sagat.networkPolicy.validatePort" (dict "name" (printf "%s.port" $portName) "port" (get $port "port")) -}}
{{- $protocol := default "TCP" (get $port "protocol") -}}
{{- if not (has $protocol (list "TCP" "UDP")) -}}
{{- fail (printf "%s.protocol must be TCP or UDP" $portName) -}}
{{- end -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{- define "sagat.networkPolicy.validate" -}}
{{- if .Values.networkPolicy.enabled -}}
{{- $egress := get .Values.networkPolicy "egress" -}}
{{- if not (kindIs "map" $egress) -}}
{{- fail "networkPolicy.egress is required when networkPolicy.enabled=true" -}}
{{- end -}}
{{- include "sagat.networkPolicy.validatePeers" (dict "name" "networkPolicy.metricsFrom" "peers" .Values.networkPolicy.metricsFrom) -}}
{{- include "sagat.networkPolicy.validatePeers" (dict "name" "networkPolicy.apiFrom" "peers" .Values.networkPolicy.apiFrom) -}}
{{- include "sagat.networkPolicy.validatePeers" (dict "name" "networkPolicy.workersFrom" "peers" .Values.networkPolicy.workersFrom) -}}
{{- include "sagat.networkPolicy.validatePeers" (dict "name" "networkPolicy.egress.dnsTo" "peers" (get $egress "dnsTo")) -}}
{{- range $dependency := list "postgres" "oidc" -}}
{{- $config := get $egress $dependency -}}
{{- if not (kindIs "map" $config) -}}
{{- fail (printf "networkPolicy.egress.%s is required" $dependency) -}}
{{- end -}}
{{- include "sagat.networkPolicy.validatePort" (dict "name" (printf "networkPolicy.egress.%s.port" $dependency) "port" (get $config "port")) -}}
{{- include "sagat.networkPolicy.validatePeers" (dict "name" (printf "networkPolicy.egress.%s.to" $dependency) "peers" (get $config "to")) -}}
{{- end -}}
{{- if and .Values.nats.url (or (eq .Values.eventTransport "") (eq .Values.eventTransport "nats")) -}}
{{- $nats := get $egress "nats" -}}
{{- if not (kindIs "map" $nats) -}}
{{- fail "networkPolicy.egress.nats is required when nats.url is configured" -}}
{{- end -}}
{{- include "sagat.networkPolicy.validatePort" (dict "name" "networkPolicy.egress.nats.port" "port" (get $nats "port")) -}}
{{- include "sagat.networkPolicy.validatePeers" (dict "name" "networkPolicy.egress.nats.to" "peers" (get $nats "to")) -}}
{{- end -}}
{{- if eq .Values.eventTransport "redis" -}}
{{- $redis := get $egress "redis" -}}
{{- if not (kindIs "map" $redis) -}}
{{- fail "networkPolicy.egress.redis is required for Redis transport" -}}
{{- end -}}
{{- include "sagat.networkPolicy.validatePort" (dict "name" "networkPolicy.egress.redis.port" "port" (get $redis "port")) -}}
{{- include "sagat.networkPolicy.validatePeers" (dict "name" "networkPolicy.egress.redis.to" "peers" (get $redis "to")) -}}
{{- end -}}
{{- include "sagat.networkPolicy.validateRules" (dict "name" "networkPolicy.egress.controlPlaneExtra" "rules" (default (list) (get $egress "controlPlaneExtra")) "peerKey" "to") -}}
{{- include "sagat.networkPolicy.validateRules" (dict "name" "networkPolicy.egress.engineExtra" "rules" (default (list) (get $egress "engineExtra")) "peerKey" "to") -}}
{{- range $poolIndex, $pool := .Values.workerPools -}}
{{- $poolPolicy := default (dict) (get $pool "networkPolicy") -}}
{{- if not (kindIs "map" $poolPolicy) -}}
{{- fail (printf "workerPools[%d].networkPolicy must be an object" $poolIndex) -}}
{{- end -}}
{{- include "sagat.networkPolicy.validateRules" (dict "name" (printf "workerPools[%d].networkPolicy.extraEgress" $poolIndex) "rules" (default (list) (get $poolPolicy "extraEgress")) "peerKey" "to") -}}
{{- include "sagat.networkPolicy.validateRules" (dict "name" (printf "workerPools[%d].networkPolicy.extraIngress" $poolIndex) "rules" (default (list) (get $poolPolicy "extraIngress")) "peerKey" "from") -}}
{{- end -}}
{{- end -}}
{{- end -}}
