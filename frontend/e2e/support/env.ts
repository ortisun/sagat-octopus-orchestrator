/** What the harness (tests/e2e/007-workflow-studio/studio_journeys.rs) tells the journeys about the platform it started. */
function need(name: string): string {
  const value = process.env[name];
  if (value === undefined || value === '') throw new Error(`${name} is not set: the journeys are run by the Rust harness (cargo test -p sagat-tests --test e2e_007 -- --ignored journeys)`);
  return value;
}

export const env = {
  origin: need('STUDIO_ORIGIN'),
  grpcUrl: need('STUDIO_GRPC_URL'),
  namespace: need('STUDIO_NAMESPACE'),
  tokens: {
    admin: need('STUDIO_TOKEN_ADMIN'),
    developer: need('STUDIO_TOKEN_DEVELOPER'),
    operator: need('STUDIO_TOKEN_OPERATOR'),
    viewer: need('STUDIO_TOKEN_VIEWER'),
    worker: need('STUDIO_TOKEN_WORKER'),
    foreign: need('STUDIO_TOKEN_FOREIGN'),
  },
};
