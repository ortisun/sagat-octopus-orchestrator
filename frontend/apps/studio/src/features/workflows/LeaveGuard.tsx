import { useContext, useState } from 'react';
import { UNSAFE_DataRouterContext, useBlocker } from 'react-router';
import { Banner } from '../../ui/Banner';
import { Button } from '../../ui/Button';
import { Modal } from './Modal';

interface Props {
  /** There is something that is not saved. */
  active: boolean;
  /** Saves, and says whether everything is saved now. */
  onSave: () => Promise<boolean>;
}

function Blocking({ active, onSave }: Props) {
  const blocker = useBlocker(active);
  const [failed, setFailed] = useState(false);
  const blocked = blocker.state === 'blocked';
  const stay = () => {
    setFailed(false);
    blocker.reset?.();
  };
  return (
    <Modal open={blocked} title="Leave without saving?" onClose={stay}>
      <p>This draft has changes that are not saved yet.</p>
      {failed ? <Banner tone="error">The draft could not be saved, so you are still here. Look at the state next to the Save button.</Banner> : null}
      <div className="dialog-actions">
        <Button onClick={stay}>Stay</Button>
        <Button variant="danger" onClick={() => blocker.proceed?.()}>
          Leave without saving
        </Button>
        <Button
          variant="primary"
          onClick={() => {
            void onSave().then((saved) => {
              if (saved) blocker.proceed?.();
              else setFailed(true);
            });
          }}
        >
          Save and leave
        </Button>
      </div>
    </Modal>
  );
}

/**
 * Asks before leaving the page with unsaved changes. Blocking a navigation needs the data router the
 * Studio runs on; under a plain router (a test, an embedding) there is nothing to ask and the
 * unload warning of the browser is what is left.
 */
export function LeaveGuard(props: Props) {
  const dataRouter = useContext(UNSAFE_DataRouterContext);
  return dataRouter ? <Blocking {...props} /> : null;
}
