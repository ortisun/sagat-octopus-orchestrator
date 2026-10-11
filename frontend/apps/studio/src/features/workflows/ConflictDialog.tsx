import { useMemo, useState } from 'react';
import { Link } from 'react-router';
import { path } from '../../api/client';
import type { Workflow } from '../../api/types';
import type { IrDocument } from '../../model/ir';
import { Banner } from '../../ui/Banner';
import { Button } from '../../ui/Button';
import { ErrorState } from '../../ui/ErrorState';
import { DiffView } from './DiffView';
import { compareDocuments } from './localDiff';
import { Modal } from './Modal';
import { openDraft } from './documentIO';

export interface CopyResult {
  id: string;
  name: string;
}

interface Props {
  open: boolean;
  /** The platform's copy: somebody saved the draft first. */
  theirs: Workflow | undefined;
  /** Set once the author's copy was saved as a workflow of its own; the conflict itself is over by then. */
  copied: CopyResult | undefined;
  /** The author's copy, still in the editor and untouched. */
  mine: IrDocument;
  onClose: () => void;
  onTakeTheirs: () => void;
  /** Saves the author's copy on top of the platform's. Resolves when it is saved or gives up. */
  onKeepMine: () => Promise<void>;
  onSaveCopy: () => Promise<void>;
}

/**
 * Two people edited the same draft (D146). Nothing is overwritten from here without the differences
 * in front of the author, and the author's copy is not touched until they choose.
 */
export function ConflictDialog({ open, theirs, copied, mine, onClose, onTakeTheirs, onKeepMine, onSaveCopy }: Props) {
  return (
    <Modal open={open} title={copied ? 'Saved as a copy' : 'Someone changed this draft'} onClose={onClose}>
      {copied ? (
        <Copied copied={copied} onClose={onClose} />
      ) : theirs ? (
        <ConflictBody theirs={theirs} mine={mine} onClose={onClose} onTakeTheirs={onTakeTheirs} onKeepMine={onKeepMine} onSaveCopy={onSaveCopy} />
      ) : null}
    </Modal>
  );
}

function Copied({ copied, onClose }: { copied: CopyResult; onClose: () => void }) {
  return (
    <div>
      <Banner tone="ok">
        Your version was saved as a new workflow, <code>{copied.name}</code>. This editor now shows the other person&apos;s draft.
      </Banner>
      <div className="dialog-actions">
        <Button onClick={onClose}>Stay here</Button>
        <Link className="btn primary" to={path`/workflows/${copied.id}`}>
          Open {copied.name}
        </Link>
      </div>
    </div>
  );
}

function ConflictBody({ theirs, mine, onClose, onTakeTheirs, onKeepMine, onSaveCopy }: Omit<Props, 'open' | 'theirs' | 'copied'> & { theirs: Workflow }) {
  const [busy, setBusy] = useState<'keep' | 'copy' | undefined>();
  const [error, setError] = useState<{ error: unknown; what: string } | undefined>();
  const diff = useMemo(() => compareDocuments(openDraft(theirs.ir).doc, mine), [theirs.ir, mine]);

  const run = async (kind: 'keep' | 'copy', what: string, action: () => Promise<void>) => {
    setBusy(kind);
    setError(undefined);
    try {
      await action();
    } catch (failure) {
      setError({ error: failure, what });
    } finally {
      setBusy(undefined);
    }
  };

  return (
    <div className="wf-conflict">
      <p>
        Another person (or another window of yours) saved this draft after you opened it, so your changes were <strong>not</strong> saved. Your copy is still here, untouched. Here
        is how their draft (revision {theirs.draftRevision}) differs from yours.
      </p>
      <DiffView diff={diff} classified={false} labels={{ from: 'their draft', to: 'your draft' }} />
      {error ? <ErrorState error={error.error} what={error.what} /> : null}
      <div className="wf-conflict-actions">
        <div>
          <Button onClick={onTakeTheirs} disabled={busy !== undefined}>
            Take theirs
          </Button>
          <p className="hint">Discard your changes and continue from their draft.</p>
        </div>
        <div>
          <Button variant="danger" onClick={() => void run('keep', 'Saving your version', onKeepMine)} disabled={busy !== undefined}>
            {busy === 'keep' ? 'Saving…' : 'Keep mine and save over theirs'}
          </Button>
          <p className="hint">Replace their draft with yours. Their changes shown above are lost from the draft.</p>
        </div>
        <div>
          <Button onClick={() => void run('copy', 'Saving your version as a copy', onSaveCopy)} disabled={busy !== undefined}>
            {busy === 'copy' ? 'Saving…' : 'Save mine as a copy'}
          </Button>
          <p className="hint">Keep your version as a new workflow and continue here from their draft.</p>
        </div>
      </div>
      <div className="dialog-actions">
        <Button onClick={onClose} disabled={busy !== undefined}>
          Decide later
        </Button>
      </div>
    </div>
  );
}
