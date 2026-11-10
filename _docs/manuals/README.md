# pii manuals

Two books, both built from the same Kaobook template and stamped with the revision they describe.

| Book | Audience | File |
|---|---|---|
| **User and Administration Manual** | whoever installs, runs and operates a pii host | `pii-user-admin.pdf` |
| **Architecture and Development Manual** | whoever changes pii's code | `pii-arch-dev.pdf` |

**These two supersede the preceding pair** — `user-manual.pdf` (19 pp) and `technical-manual.pdf`
(21 pp), with their `.tex` sources beside them. That pair is kept because `HANDOFF.md` and the
research notes cite it as evidence of an earlier state; it is history, not the manual to read. The
replacement is 85 and 104 pages respectively, with the memory subsystem documented as its own
chapter in both — the one subsystem that had no home in either earlier document.

Every book carries, on its title page, the **version**, the **build date and time**, and the
**commit** it was built from. A manual that cannot say which revision it describes is a manual
that will be wrong within a week and will not know it.

## Sources

```
_docs/manuals/src/user-admin/     src/main.tex · src/chapters/NN-name.tex · kaobook-master/
_docs/manuals/src/arch-dev/       src/main.tex · src/chapters/NN-name.tex · kaobook-master/
```

One file per chapter, numbered. The chapter list lives in `src/main.tex` as `\input` lines.

## Rebuilding

The class is Kaobook; compilation happens in the texlive container on the host, exactly as the
template's own `compile.sh` does it:

```sh
cd _docs/manuals/src/user-admin
tar czf - . | ssh <operator>@<host> "mkdir -p ~/podman/latex-remote/pii-manuals/user-admin && \
  cd ~/podman/latex-remote/pii-manuals/user-admin && tar xzf -"
ssh <operator>@<host> "podman exec --workdir /workspace/pii-manuals/user-admin/src \
  latex-compiler-shared pdflatex -interaction=nonstopmode -halt-on-error \
  -output-directory=../build main.tex"
scp <operator>@<host>:~/podman/latex-remote/pii-manuals/user-admin/build/main.pdf \
  _docs/manuals/pii-user-admin.pdf
```

Run `pdflatex` twice (the second pass settles the table of contents), then copy the PDF back.

## Two rules that cost time to learn

1. **`\begin{figure}` cannot hold a TikZ picture in this class.** Kaobook re-lays the float for
   its side column and the recursion dies with `TeX capacity exceeded [parameter stack size]`.
   Every picture is written as `\begin{center} … \captionof{figure}{…} \end{center}` with the
   `caption` package loaded. This is not a style preference; it is the only form that compiles.
2. **Figures are wrapped in `\resizebox{\linewidth}{!}{…}`.** Kaobook's text column is narrow,
   and an unwrapped diagram overflows into the margin where it is unreadable.

## What belongs in which book

- **User and administration**: install and setup; daily use; operating (backup, retention,
  upgrades, troubleshooting); the full command reference; the configuration reference; observability;
  **the graph library, graph by graph, because the graphs are what an administrator runs**.
- **Architecture and development**: design goals and their costs; the workspace and crates; the
  engine (journal, states, scheduler, recovery); node kinds and their ports; composition (the
  splice, the exit rule, pins, the capability ladder); stores and retention; the daemon, transports
  and sandbox; agents and models; gates and tests; the development workflow.

A fact appears in exactly one of them, in full, and is referenced from the other.
