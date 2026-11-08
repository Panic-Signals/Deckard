# Release notes template

Copy everything below the line into the release's description on GitHub, fill it in, and
delete the guidance in `<!-- -->`. Publishing the release builds the macOS, Linux and
Windows packages and attaches them (`.github/workflows/release.yml`).

To gather what went in, list the pull requests merged since the last release:

```
gh pr list --state merged --base main --limit 200 \
  --search "merged:>$(gh release view --json publishedAt --jq .publishedAt)" \
  --json number,author,title --jq 'sort_by(.number)[] | "#\(.number) @\(.author.login) \(.title)"'
```

GitHub's **Generate release notes** button gives the same list, sorted into the sections
below for pull requests labelled `enhancement` or `bug` (`.github/release.yml`), and
names the new contributors.

Write each line for someone using DECKARD: what they can do now, or what works now
that didn't. Link the pull request at the end of the line, and the issue it fixes.

---

DECKARD X.Y.Z: <!-- one line: the headline of this release -->

## ✨ New and Enhanced Features

<!-- Grouped under ### headings by area when there are many, each with its emoji:
🎛️ Controllers, 📚 Browser and library, 🔌 Plug-ins and mixing, 🥁 Sequencing and LFOs,
🖥️ The app. New features first, then improvements to existing ones. -->

- <!-- What you can do now. --> (#000)

## 🐛 Bug Fixes

- <!-- What works now that didn't. --> (#000, fixes #000)

## 🙏 Thank You

Thanks to everyone who contributed to this release:

<!-- Everyone with a pull request merged since the last release, and what they worked
on. Also thank people whose reports or testing led to a fix. -->

- @name: <!-- what they worked on -->

## 🎉 First-Time Contributors

<!-- Contributors whose first merged pull request is in this release. To check someone:
gh pr list --state merged --author NAME --limit 200 --json number,mergedAt \
  --jq 'sort_by(.mergedAt)[0]' -->

Welcome, and thank you for your first contribution:

- @name made their first contribution in #000

**Full changelog:** https://github.com/Panic-Signals/Deckard/compare/PREVIOUS...X.Y.Z
