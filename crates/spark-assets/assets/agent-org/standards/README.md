# Vendored standards

The established standards this seed adopts, with their provenance. A vendored file
is copied from its upstream source, either in full or in part, and is not edited
except where its header says so. To update one, replace it from the upstream
source and record the change in [CHANGELOG.md](../CHANGELOG.md).

| File | Standard | Version and source | Vendored | License |
|---|---|---|---|---|
| [rfc2119.txt](rfc2119.txt) | RFC 2119, Key words for use in RFCs to Indicate Requirement Levels (BCP 14) | <https://www.rfc-editor.org/rfc/rfc2119.txt> | In full, verbatim | IETF; the memo states "Distribution of this memo is unlimited." |
| [rfc8174.txt](rfc8174.txt) | RFC 8174, Ambiguity of Uppercase vs Lowercase in RFC 2119 Key Words (BCP 14) | <https://www.rfc-editor.org/rfc/rfc8174.txt> | In full, verbatim | IETF Trust Legal Provisions (BCP 78) |
| [semver-2.0.0.md](semver-2.0.0.md) | Semantic Versioning 2.0.0 | `semver/semver` `semver.md` at commit `d58db16863` | In full, verbatim | CC BY 3.0; specification by Tom Preston-Werner, <https://semver.org> |
| [keep-a-changelog-1.1.0.md](keep-a-changelog-1.1.0.md) | Keep a Changelog 1.1.0 | `olivierlacan/keep-a-changelog` at commit `9f482d1ef7` | In part, converted to Markdown | MIT, Olivier Lacan (notice included) |
| [make-a-readme.md](make-a-readme.md) | Make a README | `dguo/make-a-readme` at commit `8229eef74c` | In part, converted to Markdown | MIT, Danny Guo (notice included) |
| [writing-requirements.md](writing-requirements.md) | Our own guide: BCP 14 usage, ISO/IEC/IEEE 29148 requirement qualities, EARS patterns, ADRs, RACI | Written for this seed | Original | Same as this seed |

ISO/IEC/IEEE 29148, EARS, ADRs and RACI are cited and paraphrased in
[writing-requirements.md](writing-requirements.md), not copied.

## Adopted without vendoring

- **GitHub flow**, the branching model
  ([ADR-0002](../decisions/0002-git-branching-model.md)). It is described at
  <https://docs.github.com/en/get-started/using-github/github-flow>, which is
  not vendored.
