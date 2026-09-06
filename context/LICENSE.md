# thaum-painter — License

**GNU GPL v2 or later** (the Blender model) — decided 2026-09-07
(source-of-truth from J).

The painter is a free, open-source art application. Usage intent:

- free to download, use, and create with — this is an art tool, not a money thing
- distribution follows Blender's approach: the app itself stays open, so forks
  and modified builds must remain open under the same license (GPLv2+)
- artwork users create with the painter is entirely theirs — output is not
  affected by the app's license
- commercial use of the app is permitted under GPL terms

## Before publishing

- Drop the full GPLv2 text (from https://www.gnu.org/licenses/gpl-2.0.txt)
  into a repo-root `LICENSE` file; this context file records the decision and
  rationale, the repo root carries the legal text.
- Copyright holder: **j art and design** (decided 2026-09-07).
- The renderer (`thaum-renderer`) is separately MIT-licensed so it can be
  embedded in any project, including closed ones; the painter depending on it
  is fine (GPL app + MIT library is a valid one-directional pairing).
- Typeface/atlas provenance audit still pending before the repos go public.
