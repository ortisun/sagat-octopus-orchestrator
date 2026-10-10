import { useQuery } from '@tanstack/react-query';
import { useState } from 'react';
import { path } from '../../api/client';
import type { components } from '../../api/schema';
import { useApp, useCan, useNamespace } from '../../app/context';
import { setField } from '../../model/edit';
import type { IrNode } from '../../model/ir';
import { ErrorState } from '../../ui/ErrorState';
import { Field } from '../../ui/Field';
import type { Change } from './fields';

type ScriptList = components['schemas']['ScriptList'];
type ScriptVersionList = components['schemas']['ScriptVersionList'];

/**
 * Picks the built version of one of the namespace's scripts for a script node: the node stores the
 * digest of the build, so the step runs exactly that code. Pasting a digest stays possible below it.
 */
export function ScriptPicker({ node, disabled, change }: { node: IrNode; disabled: boolean; change: Change }) {
  const { ep, api } = useApp();
  const slug = useNamespace();
  const canRead = useCan('script:read');
  const [scriptId, setScriptId] = useState('');

  const namespaces = useQuery({ queryKey: ['namespaces'], queryFn: ({ signal }) => ep.namespaces(signal) });
  const namespaceId = namespaces.data?.items.find((n) => n.slug === slug)?.id;

  const scripts = useQuery({
    queryKey: ['scripts', namespaceId],
    enabled: canRead && namespaceId !== undefined,
    queryFn: ({ signal }) => api.get<ScriptList>(path`/namespaces/${namespaceId ?? ''}/scripts`, { query: { limit: 200 }, signal }),
  });
  const versions = useQuery({
    queryKey: ['script-versions', namespaceId, scriptId],
    enabled: canRead && namespaceId !== undefined && scriptId !== '',
    queryFn: ({ signal }) => api.get<ScriptVersionList>(path`/namespaces/${namespaceId ?? ''}/scripts/${scriptId}/versions`, { query: { limit: 50 }, signal }),
  });

  if (!canRead) {
    return <p className="hint">Picking a script needs the script:read permission. You can still paste the digest of a build below.</p>;
  }
  if (namespaces.isError) return <ErrorState error={namespaces.error} what="Loading the namespace" />;
  if (scripts.isError) return <ErrorState error={scripts.error} what="Loading the scripts" onRetry={() => void scripts.refetch()} />;

  const ready = (versions.data?.items ?? []).filter((v) => v.status === 'READY' && typeof v.artifactDigest === 'string');
  const current = typeof node['artifactDigest'] === 'string' ? node['artifactDigest'] : '';

  return (
    <div className="wf-picker">
      <Field label="Script" hint={scripts.isPending ? 'Loading the scripts…' : (scripts.data?.items.length ?? 0) === 0 ? 'This namespace has no scripts yet.' : undefined}>
        <select value={scriptId} disabled={disabled || scripts.isPending} onChange={(event) => setScriptId(event.target.value)}>
          <option value="">(choose a script)</option>
          {(scripts.data?.items ?? []).map((s) => (
            <option key={s.id} value={s.id}>
              {s.name} ({s.language})
            </option>
          ))}
        </select>
      </Field>
      {scriptId !== '' ? (
        <>
          {versions.isError ? <ErrorState error={versions.error} what="Loading the versions" onRetry={() => void versions.refetch()} /> : null}
          <Field
            label="Built version"
            hint={
              versions.isPending
                ? 'Loading the versions…'
                : ready.length === 0
                  ? 'This script has no version that built successfully.'
                  : 'The node keeps the digest of the build you pick.'
            }
          >
            <select
              value={ready.find((v) => v.artifactDigest === current)?.artifactDigest ?? ''}
              disabled={disabled || versions.isPending}
              onChange={(event) => {
                const chosen = ready.find((v) => v.artifactDigest === event.target.value);
                if (!chosen) return;
                change((doc) => setField(setField(doc, node.id, 'artifactDigest', chosen.artifactDigest), node.id, 'language', chosen.language), `script:${node.id}`);
              }}
            >
              <option value="">(choose a version)</option>
              {ready.map((v) => (
                <option key={v.id} value={v.artifactDigest}>
                  v{v.number} · {v.language}
                </option>
              ))}
            </select>
          </Field>
        </>
      ) : null}
    </div>
  );
}
