# Tapes for the book's GIFs

Each `.tape` file is a [VHS](https://github.com/charmbracelet/vhs) script
that records one GIF for the user guide. The GIFs go to `../src/images/`.

| Tape | Shows |
| --- | --- |
| `filter-stack.tape` | exclude and include filters, hide mode and the peek |
| `browsing.tape` | the explorer marks, and hide mode on a directory |
| `filter-editor.tape` | live matches, and a failed check that a pattern change corrects |
| `command-view.tape` | a JSON file through `jq`, and a filter on the jq lines |
| `batch.tape` | batch mode in the shell |

## Make the GIFs again

Install VHS (`brew install vhs`), then from the repository root:

```sh
docs/book/tapes/make-gifs.sh                                  # every tape
docs/book/tapes/make-gifs.sh docs/book/tapes/browsing.tape    # one tape
docs/book/tapes/make-gifs.sh --check                          # validate only
```

The script builds `target/release/recon` and puts it first on `PATH`. It
copies `data/` to `/tmp/recon-demo` and `config/` to `/tmp/recon-demo-config`,
so that each GIF shows the same short paths on each computer. It runs VHS with
a clean environment: only `HOME`, `PATH` and `XDG_CONFIG_HOME` (the demo
config). Thus your own config and your `RECON_*` or `EDITOR` settings do not
change the GIF.

## Rules for a tape

- Start with `Output`, then `Set Width` and `Set Height`, then
  `Source docs/book/tapes/common.tape`. VHS ignores a `Set` after the hidden
  `cd` in `common.tape`.
- Use the default keys (`recon --print-keymap defaults`).
- Keep each GIF short. The script refuses a GIF over 5 MB.

`make-gifs.test.sh` tests the script.

## Limits

VHS draws the terminal with xterm.js, so the image and Quick Look views do
not show. Record those with a screen recording of a real terminal.
