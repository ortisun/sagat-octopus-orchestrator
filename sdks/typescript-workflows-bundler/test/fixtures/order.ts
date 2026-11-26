import { WorkflowFailure, type WorkflowContext } from '@sagat-octopus-orchestrator/sdk/workflows';
import { priceOf } from './pricing.js';

export async function order(ctx: WorkflowContext, input: { sku: string; quantity: number }): Promise<unknown> {
  const reserved = await ctx.activity('inventory.reserve', input);
  const amount = priceOf(input.quantity);
  if (amount <= 0) throw new WorkflowFailure('EMPTY_ORDER', 'nothing to charge');
  const charged = await ctx.activity('payments.charge', { amount });
  await ctx.sleep(86_400_000);
  return { reserved, charged, id: ctx.uuid() };
}

export async function refund(ctx: WorkflowContext, input: { amount: number }): Promise<unknown> {
  return ctx.activity('payments.refund', input);
}

export const notAWorkflow = 42;
