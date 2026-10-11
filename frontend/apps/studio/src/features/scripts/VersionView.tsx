import { useQuery } from '@tanstack/react-query';
import { useEffect, useRef } from 'react';
import { Banner } from '../../ui/Banner';
import { Button } from '../../ui/Button';
import { ErrorState } from '../../ui/ErrorState';
import { StatusBadge } from '../../ui/StatusBadge';
import type { ScriptsApi } from './api';
import { CodeEditor } from './CodeEditor';
import { CopyButton } from './CopyButton';
import { schemaToText } from './jsonText';
import { languageInfo, monacoLanguage } from './languages';
import { LogView } from './LogView';
import { keys } from './queries';
import { buildFailureHelp, versionStatus } from './status';
import { clipLabel, formatTime } from './text';
import { LimitsPanel } from './LimitsPanel';

/** A published version as it was published: read-only, with its digest, its build log and its limits. */
export function VersionView({ api, scriptId, number, onBack }: { api: ScriptsApi; scriptId: string; number: number; onBack: () => void }) {
  const query = useQuery({
    queryKey: keys.version(api.namespaceId, scriptId, number),
    queryFn: ({ signal }) => api.version(scriptId, number, signal),
    // A version never changes once it is built; while it builds, it is read again by the list.
    staleTime: 10_000,
  });
  const heading = useRef<HTMLHeadingElement>(null);
  // Opening a version moves the reader to it.
  useEffect(() => {
    heading.current?.focus();
  }, []);
  const version = query.data;
  const info = languageInfo(version?.language ?? '');
  return (
    <section className="version-view" aria-labelledby="version-title">
      <div className="toolbar">
        <Button onClick={onBack}>Back to the draft</Button>
      </div>
      <h2 id="version-title" ref={heading} tabIndex={-1}>
        Version {number} <span className="hint">(read-only)</span>
      </h2>
      {query.isPending ? <p role="status">Loading version {number}…</p> : null}
      {query.isError ? <ErrorState error={query.error} what={`Reading version ${number}`} onRetry={() => void query.refetch()} /> : null}
      {version ? (
        <>
          <Banner tone="info">This is version {version.number} as it was published. A version never changes; your draft is untouched.</Banner>
          <dl className="numbers">
            <dt>State</dt>
            <dd>
              <StatusBadge view={versionStatus(version.status)} />
            </dd>
            <dt>Artifact digest</dt>
            <dd>
              {version.artifactDigest ? (
                <>
                  <code>{version.artifactDigest}</code> <CopyButton text={version.artifactDigest} label="Copy digest" what={`the digest of version ${version.number}`} />
                  <p className="hint">Use this digest in a Script node of a workflow.</p>
                </>
              ) : (
                <span className="hint">{version.status === 'BUILDING' ? 'The build has not finished.' : 'The build made no artifact.'}</span>
              )}
            </dd>
            {version.activity ? (
              <>
                <dt>Activity</dt>
                <dd>
                  <code>{clipLabel(version.activity, 100)}</code>
                </dd>
              </>
            ) : null}
            <dt>Language</dt>
            <dd>{info.label}</dd>
            <dt>Published by</dt>
            <dd>
              {clipLabel(version.createdBy)} at <time dateTime={version.createdAt}>{formatTime(version.createdAt)}</time>
            </dd>
            {version.builtAt ? (
              <>
                <dt>Built</dt>
                <dd>
                  <time dateTime={version.builtAt}>{formatTime(version.builtAt)}</time>
                </dd>
              </>
            ) : null}
            {version.toolchain ? (
              <>
                <dt>Toolchain</dt>
                <dd>
                  <code>{clipLabel(version.toolchain, 120)}</code>
                </dd>
              </>
            ) : null}
          </dl>
          {version.failure ? (
            <Banner tone="error" title="The build failed.">
              <code>{clipLabel(version.failure, 300)}</code> {buildFailureHelp(version.failure)}
            </Banner>
          ) : null}
          {version.buildLog ? (
            <>
              <h3>Build log</h3>
              <LogView text={version.buildLog} label={`Build log of version ${version.number}`} empty="(empty)" />
            </>
          ) : null}
          <h3>Source</h3>
          <CodeEditor label={`Source of version ${version.number} (read-only)`} model="version" language={monacoLanguage(version.language)} value={version.source} readOnly height="20rem" />
          {version.lockfile ? (
            <>
              <h3>Lockfile</h3>
              <LogView text={version.lockfile} label="Lockfile" empty="(empty)" />
            </>
          ) : null}
          <h3>Schemas</h3>
          <div className="schema-pair">
            <div>
              <h4>Input schema</h4>
              <LogView text={schemaToText(version.inputSchema)} label="Input schema" empty="(none)" />
            </div>
            <div>
              <h4>Output schema</h4>
              <LogView text={schemaToText(version.outputSchema)} label="Output schema" empty="(none)" />
            </div>
          </div>
          <h3>Limits</h3>
          <LimitsPanel policy={version.policy} retentionDays={version.retentionDays} language={info.label} />
        </>
      ) : null}
    </section>
  );
}
