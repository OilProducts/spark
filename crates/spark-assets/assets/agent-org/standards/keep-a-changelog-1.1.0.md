# Keep a Changelog 1.1.0 (vendored in part)

> Source: <https://keepachangelog.com/en/1.1.0/>, from
> `olivierlacan/keep-a-changelog` at commit `9f482d1ef7`
> (`source/en/1.1.0/index.html.haml`). Converted from HAML to Markdown. The
> example changelog, the FAQ and the contribution sections are omitted.
> Everything else below is the original wording.
>
> Copyright (c) 2014 Olivier Lacan. Licensed under the MIT License. The full
> license text is at the end of this file.

## What is a changelog?

A changelog is a file which contains a curated, chronologically ordered list of
notable changes for each version of a project.

## Why keep a changelog?

To make it easier for users and contributors to see precisely what notable
changes have been made between each release (or version) of the project.

## Who needs a changelog?

People do. Whether consumers or developers, the end users of software are human
beings who care about what's in the software. When the software changes, people
want to know why and how.

## How do I make a good changelog?

### Guiding Principles

- Changelogs are *for humans*, not machines.
- There should be an entry for every single version.
- The same types of changes should be grouped.
- Versions and sections should be linkable.
- The latest version comes first.
- The release date of each version is displayed.
- Mention whether you follow [Semantic Versioning](semver-2.0.0.md).

### Types of changes

- `Added` for new features.
- `Changed` for changes in existing functionality.
- `Deprecated` for soon-to-be removed features.
- `Removed` for now removed features.
- `Fixed` for any bug fixes.
- `Security` in case of vulnerabilities.

## How can I reduce the effort required to maintain a changelog?

Keep an `Unreleased` section at the top to track upcoming changes.

This serves two purposes:

- People can see what changes they might expect in upcoming releases
- At release time, you can move the `Unreleased` section changes into a new
  release version section.

## Can changelogs be bad?

Yes. Here are a few ways they can be less than useful.

### Commit log diffs

Using commit log diffs as changelogs is a bad idea: they're full of noise.
Things like merge commits, commits with obscure titles, documentation changes,
etc.

The purpose of a commit is to document a step in the evolution of the source
code. Some projects clean up commits, some don't.

The purpose of a changelog entry is to document the noteworthy difference,
often across multiple commits, to communicate them clearly to end users.

### Ignoring Deprecations

When people upgrade from one version to another, it should be painfully clear
when something will break. It should be possible to upgrade to a version that
lists deprecations, remove what's deprecated, then upgrade to the version where
the deprecations become removals.

If you do nothing else, list deprecations, removals, and any breaking changes in
your changelog.

### Confusing Dates

Regional date formats vary throughout the world and it's often difficult to find
a human-friendly date format that feels intuitive to everyone. The advantage of
dates formatted like `2017-07-17` is that they follow the order of largest to
smallest units: year, month, and day. This format also doesn't overlap in
ambiguous ways with other date formats, unlike some regional formats that switch
the position of month and day numbers. These reasons, and the fact this date
format is an ISO standard, are why it is the recommended date format for
changelog entries.

### Inconsistent Changes

A changelog which only mentions some of the changes can be as dangerous as not
having a changelog. While many of the changes may not be relevant - for
instance, removing a single whitespace may not need to be recorded in all
instances - any important changes should be mentioned in the changelog. By
inconsistently applying changes, your users may mistakenly think that the
changelog is the single source of truth. It ought to be. With great power comes
great responsibility - having a good changelog means having a consistently
updated changelog.

## What about yanked releases?

Yanked releases are versions that had to be pulled because of a serious bug or
security issue. Often these versions don't even appear in change logs. They
should. This is how you should display them:

`## [0.0.5] - 2014-12-13 [YANKED]`

The `[YANKED]` tag is loud for a reason. It's important for people to notice it.
Since it's surrounded by brackets it's also easier to parse programmatically.

## Should you ever rewrite a changelog?

Sure. There are always good reasons to improve a changelog. I regularly open
pull requests to add missing releases to open source projects with unmaintained
changelogs.

It's also possible you may discover that you forgot to address a breaking change
in the notes for a version. It's obviously important for you to update your
changelog in this case.

---

The MIT License (MIT)

Copyright (c) 2014 Olivier Lacan

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
