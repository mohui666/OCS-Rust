# Userscript State Regressions

Run `pnpm test:userscript /absolute/path/to/chromium` after installing workspace dependencies.
Alternatively, install the test browser with `pnpm --filter @ocs-desktop/app exec playwright-core install chromium` and run `pnpm test:userscript`.
`OCS_TEST_CHROMIUM` can also supply an existing executable. Generated reports go to `.cache/userscript-tests/`.

These tests use synthetic pages, fake answer providers, and virtual clocks. They do not connect to a real model, use course accounts, or submit coursework.

## Failure Mechanism

The extension stores a whole tab object. Concurrent read/modify/write operations for results and progress could each read an old snapshot, then overwrite the other update. The result panel also subscribed to the wrong change-notification channel, and asynchronous adapter callbacks did not await persistence. This allowed correct option selections to coexist with stale hollow question numbers.

Additional lifecycle tests reproduce out-of-order panel reads, updates missed while a subscription is registering, premature completion before the final save, and callbacks queued by a closed task. Tests are run against the distributed userscript, not a separate reimplementation.

## Guarded Invariants

- Whole-tab writes are serialized within each script instance; later reads wait for already queued writes. Unrelated tab data survives and a failed write does not poison the queue.
- Results use tab notifications. Panels re-read after subscription and discard reads superseded by newer refreshes.
- Result callbacks await persistence. Closed workers skip queued callbacks; completion waits for the final progress save and propagates save failure.
- Synthetic 85-question runs compare actual input values, stored results, number styles, and counts. An uncertain answer remains unfilled.
- Pause/resume, delayed responses, unlimited waits, mixed question types, and 200-question batches remain covered.

The GitHub workflow runs this command for relevant source changes. Local success is not proof of remote CI success, model accuracy, server-side course saving, or cross-frame atomicity. This queue does not provide a lock across separate script instances.
