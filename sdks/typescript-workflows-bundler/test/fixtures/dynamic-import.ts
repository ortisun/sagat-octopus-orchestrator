import type { WorkflowContext } from '@sagat-octopus-orchestrator/sdk/workflows';

export async function dynamic(ctx: WorkflowContext, input: { module: string }): Promise<unknown> {
  const pricing = await import('./pricing.js');
  try {
    await import(input.module);
    return 'imported';
  } catch (error) {
    return { price: pricing.priceOf(2), refused: (error as Error).name, at: ctx.now().getTime() > 0 };
  }
}
