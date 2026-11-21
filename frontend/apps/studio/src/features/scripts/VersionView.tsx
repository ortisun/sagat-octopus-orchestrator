import { useQuery } from '@tanstack/react-query';
import { useEffect, useRef } from 'react';
import { Banner, Button, Code, ErrorState, Facts, FocusHeading, Heading, Hint, Loading, Row, Stack, StatusBadge, Text } from '../../ui';
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
    <section aria-labelledby="version-title">
      <Stack gap={3}>
        <Row>
          <Button onClick={onBack}>Back to the draft</Button>
        </Row>
        <FocusHeading level={2} id="version-title" ref={heading}>
          Version {number}{' '}
          <Text as="span" muted>
            (read-only)
          </Text>
        </FocusHeading>
        {query.isPending ? <Loading>{`Loading version ${number}…`}</Loading> : null}
        {query.isError ? <ErrorState error={query.error} what={`Reading version ${number}`} onRetry={() => void query.refetch()} /> : null}
        {version ? (
          <>
            <Banner tone="info">This is version {version.number} as it was published. A version never changes; your draft is untouched.</Banner>
            <Facts
              items={[
                { term: 'State', value: <StatusBadge view={versionStatus(version.status)} /> },
                {
                  term: 'Artifact digest',
                  value: version.artifactDigest ? (
                    <Stack gap={1}>
                      <Row>
                        <Code>{version.artifactDigest}</Code>
                        <CopyButton text={version.artifactDigest} label="Copy digest" what={`the digest of version ${version.number}`} />
                      </Row>
                      <Hint>Use this digest in a Script node of a workflow.</Hint>
                    </Stack>
                  ) : (
                    <Text as="span" muted>
                      {version.status === 'BUILDING' ? 'The build has not finished.' : 'The build made no artifact.'}
                    </Text>
                  ),
                },
                ...(version.activity ? [{ term: 'Activity', value: <Code>{clipLabel(version.activity, 100)}</Code> }] : []),
                { term: 'Language', value: info.label },
                {
                  term: 'Published by',
                  value: (
                    <>
                      {clipLabel(version.createdBy)} at <time dateTime={version.createdAt}>{formatTime(version.createdAt)}</time>
                    </>
                  ),
                },
                ...(version.builtAt ? [{ term: 'Built', value: <time dateTime={version.builtAt}>{formatTime(version.builtAt)}</time> }] : []),
                ...(version.toolchain ? [{ term: 'Toolchain', value: <Code>{clipLabel(version.toolchain, 120)}</Code> }] : []),
              ]}
            />
            {(version.failureCode || version.failure) ? (
              <Banner tone="error" title="The build failed.">
                <Code>{clipLabel(version.failureCode || version.failure || '', 300)}</Code> {buildFailureHelp(version.failureCode ?? version.failure)}
              </Banner>
            ) : null}
            {version.buildLogTruncated ? <Hint role="status">Build diagnostics were truncated by the retained-log limit.</Hint> : null}
            {version.buildLog ? (
              <Stack gap={1}>
                <Heading level={3}>Build log</Heading>
                <LogView text={version.buildLog} label={`Build log of version ${version.number}`} empty="(empty)" />
              </Stack>
            ) : null}
            <Stack gap={1}>
              <Heading level={3}>Source</Heading>
              <CodeEditor label={`Source of version ${version.number} (read-only)`} model="version" language={monacoLanguage(version.language)} value={version.source} readOnly height="20rem" />
            </Stack>
            {version.lockfile ? (
              <Stack gap={1}>
                <Heading level={3}>Lockfile</Heading>
                <LogView text={version.lockfile} label="Lockfile" empty="(empty)" />
              </Stack>
            ) : null}
            <Heading level={3}>Schemas</Heading>
            <div className="schema-pair">
              <Stack gap={1}>
                <Heading level={4}>Input schema</Heading>
                <LogView text={schemaToText(version.inputSchema)} label="Input schema" empty="(none)" />
              </Stack>
              <Stack gap={1}>
                <Heading level={4}>Output schema</Heading>
                <LogView text={schemaToText(version.outputSchema)} label="Output schema" empty="(none)" />
              </Stack>
            </div>
            <Heading level={3}>Limits</Heading>
            <LimitsPanel policy={version.policy} retentionDays={version.retentionDays} language={info.label} />
          </>
        ) : null}
      </Stack>
    </section>
  );
}
