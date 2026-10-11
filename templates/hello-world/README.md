# hello-world

Two steps: `greeting.compose`, then `greeting.deliver`.

```sh
sagat workflow validate workflow.json
sagat workflow publish workflow.json
sagat worker dev --mocks mocks.yaml --workflow workflow.json --bind      # in another terminal; leave it running
sagat workflow run hello-world --input '{"name":"Ada"}' --wait
```

To serve the activities with an SDK instead of `worker dev`, see `docs/quickstarts/`.
