# CLAUDE.md

## Design principles

- **No custom flags.** All tool flags must mirror real Unix command flags. If a Unix command doesn't have a flag for the behavior we need, find the closest Unix equivalent (e.g. `-maxdepth` instead of `-limit`). Never invent new flags — discoverability comes from agents and users already knowing the Unix toolchain.
