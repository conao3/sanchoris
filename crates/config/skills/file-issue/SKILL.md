---
name: file-issue
description: File a GitHub issue for work that belongs to another task. Run it when the work should happen in its own session, for example in another repository; the issue's trigger starts that task and the chat shows it.
---

# file-issue

This is a reliable skill. Sanchoris runs it outside your session: it opens one issue in the repository you name, with the title, body, and labels you give, and records the issue so that the chat shows the task that the issue starts. Running it twice with the same input in the same skill run does not open a second issue.

## When to run it

Run it once per piece of work that should be done separately, after you know which repository it belongs to. Write the body so that an agent who has not seen this conversation can do the work from it. Add the labels that the repository's trigger reacts to (for example `sanchoris`), or the issue starts no task.

## How to run it

```sh
sanchoris skill run file-issue --input '{"Repo": "acme/shop-web", "Title": "Add a checkout spec", "Body": "The checkout flow has no spec. Add one that covers ...", "Labels": ["sanchoris"]}'
```

- `Repo`: the repository to open the issue in, as `owner/name`. It must be in the session's `GitHub.Repos`.
- `Title`: the issue title.
- `Body`: the issue body in Markdown.
- `Labels`: the labels to add, as a list of names; `[]` for none.

The command returns at once with `{ "RunId": <id>, "Status": "RUNNING" }`. Do not wait or poll. End your turn or continue with other work.

## What happens next

Sanchoris sends you a message that starts with `[sanchoris]`:

- `SUCCEEDED`: the issue is open. The output has its `Repo`, `Number`, and `Url`.
- `FAILED`: the issue was not opened, for example because the repository is outside the session or the session cannot write issues. Report the cause.
