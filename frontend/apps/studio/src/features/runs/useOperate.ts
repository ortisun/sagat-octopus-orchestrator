import { useCan } from '../../app/context';

/** What the signed-in person may do about runs, as the platform told the Studio (the platform still decides each request). */
export function useCanOperate(): (permission: string) => boolean {
  const operate = useCan('run:operate');
  const override = useCan('saga:override');
  return (permission) => (permission === 'run:operate' ? operate : permission === 'saga:override' ? override : false);
}
