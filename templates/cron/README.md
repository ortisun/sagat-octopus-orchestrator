# cron

A run every day at 02:00 in `Europe/Lisbon`. Change `cron` (five fields) and `timezone` (an IANA name).

```sh
sagat workflow publish workflow.json
sagat worker dev --mocks mocks.yaml --workflow workflow.json --bind
```

The runs it started: `sagat workflow runs nightly-report`. A run can also be started by hand: `sagat workflow run nightly-report --wait`.
