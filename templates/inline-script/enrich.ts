// A script runs in a sandbox the platform builds and signs: no network, no files, no secrets.
// It reads the step's input and returns the step's output. See docs/runbooks/scripts.md.
export default function enrich(input: { orderId: string; items: number }) {
  return { orderId: input.orderId, items: input.items, express: input.items <= 3 };
}
