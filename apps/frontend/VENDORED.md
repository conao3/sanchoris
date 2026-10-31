# Code adapted from T3 Code

The chat timeline and the question composer are adapted from [T3 Code](https://github.com/pingdotgg/t3code) at commit `044f07257c9684ae4543bd1e1ebf9c394e9e3830`, under the MIT License below. T3 Code builds them on its own state store and UI kit; the parts used here are rewritten on this app's components and Tailwind tokens. No file is copied verbatim.

| File here | Adapted from | What was taken |
|---|---|---|
| `src/components/chat/Timeline.tsx` | `apps/web/src/components/chat/TimelineUserMessageRow.tsx`, `TimelineAssistantRows.tsx`, `TimelineWorkGroupRows.tsx`, `TimelineWorkEntryRow.tsx`, `MessagesTimeline.logic.ts` | A person's message as a right-aligned bubble and the agent's reply as full-width Markdown with the time and a copy button on hover; the work of a finished turn folded into "Worked for …"; consecutive tool calls and thoughts grouped into one row with a summary, expanding into rows that open the command, its output, or the full thought; the live row "Working for …" with the latest action |
| `src/components/chat/work.ts` | `packages/client-runtime/src/work-log/presentation.ts`, `apps/web/src/components/chat/TimelineWorkEntryRow.logic.ts` | Tool classification by name (command, read, edit, search, web, agent), the group summary sentence such as "Read 3 files and ran 2 commands", the group icon, and the elapsed time format |
| `src/components/chat/QuestionComposer.tsx` | `apps/web/src/components/chat/ComposerPendingUserInputPanel.tsx`, `apps/web/src/pendingUserInput.ts`, `ComposerPrimaryActions.tsx` | One question at a time with its header and "1/3", options as full-width buttons with number keys and a check mark, single-select advancing after 200 ms, multi-select toggling, the composer text as the person's own answer taking precedence over the picked option, and the button copy "Next question" / "Submit answer(s)" |
| `src/components/chat/Markdown.tsx` | `apps/web/src/components/ChatMarkdown.tsx` | `react-markdown` with `remark-gfm` for the agent's replies and thoughts |

## License

```text
MIT License

Copyright (c) 2026 T3 Tools Inc.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
