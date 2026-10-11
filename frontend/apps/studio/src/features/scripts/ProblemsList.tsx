import type { RefObject } from 'react';
import { StatusBadge } from '../../ui/StatusBadge';
import type { StatusView } from '../../model/status';
import type { EditorControls } from './CodeEditor';
import type { Problem, ProblemSeverity } from './monacoSetup';

const SEVERITY: Record<ProblemSeverity, StatusView> = {
  error: { label: 'Error', symbol: '✕', tone: 'bad', help: 'The language service says this is wrong.' },
  warning: { label: 'Warning', symbol: '△', tone: 'waiting', help: 'The language service has doubts about this.' },
  info: { label: 'Note', symbol: 'ℹ', tone: 'idle', help: 'A remark of the language service.' },
};

/**
 * What the editor underlines, as a list: a squiggle is not something a keyboard or a screen reader
 * can find. Each problem says where it is and takes the cursor there.
 */
export function ProblemsList({ problems, controlsRef, label }: { problems: readonly Problem[]; controlsRef: RefObject<EditorControls | null>; label: string }) {
  const errors = problems.filter((problem) => problem.severity === 'error').length;
  return (
    <div className="problems">
      <h3 id={`${label}-problems`}>
        Problems{' '}
        <span className="count" aria-live="polite">
          {problems.length === 0 ? 'none reported' : `${errors} ${errors === 1 ? 'error' : 'errors'}, ${problems.length - errors} other`}
        </span>
      </h3>
      {problems.length > 0 ? (
        <ul aria-labelledby={`${label}-problems`}>
          {problems.map((problem, index) => (
            <li key={`${problem.line}:${problem.column}:${index}`}>
              <StatusBadge view={SEVERITY[problem.severity]} />{' '}
              <button type="button" className="link-button" onClick={() => controlsRef.current?.reveal(problem.line, problem.column)}>
                Line {problem.line}, column {problem.column}
              </button>{' '}
              <span>{problem.message}</span>
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}
