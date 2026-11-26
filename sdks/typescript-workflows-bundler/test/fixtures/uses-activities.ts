import { charge } from './activities/payments.js';

export async function leak(): Promise<unknown> {
  return charge();
}
