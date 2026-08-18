---
spec: book-chapter
name: "Core Concepts: Events & Actions"
chapter: "第二部分：核心概念"
target_language: zh
source_language: en
tags: [makepad, book, events, actions, zh, spec-driven-writing]
---

## Intent

Complete the Chinese chapter `src-zh/core-concepts/events.md` for the bilingual Makepad book.

The chapter must become the primary Chinese explanation of Makepad's event and action model, based on the existing English source chapter `src/core-concepts/events.md`, while preserving the English chapter unchanged.

This is a book-writing task, not a literal translation task. The Chinese chapter should read as a polished engineering book chapter for Chinese readers, using the `tech-writer` and `trilingual-collab` rules.

## Non-Goals

- Do not delete or rewrite the English source chapter.
- Do not expand into unrelated Makepad subsystems beyond what is needed to explain events/actions.
- Do not add unsupported claims, URLs, or APIs not present in the source chapter or the referenced repository code.
- Do not polish every chapter in the book in this pass.

## User-Facing Outcome

After this task, a Chinese reader should be able to:

- Understand the difference between raw events and semantic actions.
- Explain the respective roles of `AppMain::handle_event` and `MatchEvent::handle_actions`.
- Follow the frame-local flow from OS event → widget handling → queued action → app logic.
- Use common helper methods like `clicked()`, `changed()`, and `returned()`.
- Recognize when to use timers and `NextFrame` for animation or deferred work.
- Know how events propagate to child widgets in custom containers.

## Language Policy

- Chinese is the default authoring language.
- English content is preserved as source material and may remain in `src/`.
- Chinese chapter content should be natural technical Chinese, not translationese.
- Follow `skills/trilingual-collab/references/zh.md` for final polish.

## Source Material

Primary source chapter:

- `examples/book/src/core-concepts/events.md`

Supporting source material allowed if needed for verification:

- Repository code in `examples/aichat/`
- Makepad widget code referenced by the chapter

## Required Style Constraints

Follow `skills/tech-writer/SKILL.md` and `skills/tech-writer/references/templates.md` for book chapters:

- Start with why this chapter matters.
- Prefer conclusion-first explanations.
- Use source-backed claims only.
- Use concise code excerpts where they clarify the model.
- Avoid duplicated arguments and motivational filler.

Follow `skills/trilingual-collab/references/zh.md`:

- 中文排版使用全角标点。
- 中英文之间保留空格。
- 避免翻译腔、互联网黑话、清嗓子开场和总结复述结尾。
- 加粗只用于首次定义或真正必要的术语强调。

## Chapter Budget

Type: book-chapter

Target length:

- Chinese正文约 1200–1800 字
- 代码块不计入严格字数，但应控制在必要范围内

Depth:

- Explain the event/action split clearly.
- Explain one full event flow end to end.
- Cover common event types, timers/animation, and child propagation at a practical level.

## Acceptance Criteria

- `src-zh/core-concepts/events.md` is no longer an outline stub.
- The Chinese chapter preserves the technical meaning of the English chapter while improving readability for Chinese readers.
- The chapter includes at least:
  - a short introduction explaining why events/actions matter in Makepad
  - a comparison of `AppMain` and `MatchEvent`
  - an event-flow explanation
  - code examples for handling widget actions
  - a short section on timers / `NextFrame`
  - a short section on propagating events to children
- `src/core-concepts/events.md` remains unchanged.
- Default book entry is switched to Chinese after or alongside this work, while keeping `book-en.toml` available.

## Suggested Verification

- Manually compare the finished Chinese chapter against the English source chapter for omissions.
- Run the mdBook Chinese build command used by this book if available.
