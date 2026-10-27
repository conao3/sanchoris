---
name: open-pr-green-ci
description: Push your branch, open pull requests, and get CI green. Run it once your change is committed and you want it reviewed; it reports back when the checks pass or hands CI failures to you to fix.
---

# open-pr-green-ci

This is a reliable skill. Sanchoris runs it outside your session: it pushes the task branch, opens or updates a pull request in each repository, waits for the checks, and comments on the issue once every check passes.

## When to run it

Run it after you have committed the change for the task in every repository you touched. Do not open pull requests yourself with `gh` or `git push`; this skill does it.

## How to run it

```sh
sanchoris skill run open-pr-green-ci --input '{"Repos": ["acme/shop-web"], "Repo": "acme/shop-web", "Issue": 35}'
```

- `Repos`: the repositories to push and open pull requests in, as `owner/name`.
- `Repo`: the repository of the issue to comment on, as `owner/name`.
- `Issue`: the issue number to comment on.

The command returns at once with `{ "RunId": <id>, "Status": "RUNNING" }`. Do not wait or poll. End your turn.

## What happens next

Sanchoris sends you a message that starts with `[sanchoris]`:

- `SUCCEEDED`: the pull requests are open and the checks pass. Summarize the result and finish.
- `NEEDS_AGENT`: CI failed. The message contains the failing checks and the log. Fix the cause, commit, and run `sanchoris skill continue <id> --output '{}'` with the run ID from the message. The skill pushes again and waits for the checks.
- `FAILED`: the skill gave up, for example `Skill.FixesExhausted` after 3 fixes. Explain what still fails and stop.
