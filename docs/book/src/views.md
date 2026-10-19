# About views

recon started as a log viewer, but not every file is a log. A JSON file is
easier to read after `jq`. A photo has its data in EXIF fields. A PDF is not
text. *Views* let recon show each file in the form that makes it readable.

## The view cycle

A *view* is one way to show a file. The built-in views are:

| View | Shows |
| --- | --- |
| `raw` | the text of the file |
| `hex` | a hex dump |
| `image` | the file as a picture |
| `quicklook` | on macOS, a Quick Look preview |

Each file type has a *view cycle*: an ordered list of views. The first view
is the default, and `-` goes to the next view. A text file starts in `raw`, a
binary file in `hex`, and a picture in `image`. A `[[filetype]]` row in the
configuration sets the cycle for a file type.

## Command views

A *command view* shows a file as the output of a program that you name in
the configuration:

```toml
[commands]
jq   = ["jq", "."]
exif = ["exiftool", "{file}"]

[[filetype]]
glob  = "*.json"
views = ["jq", "raw", "hex"]
```

The program is yours, not part of recon. Thus a command view makes recon
extensible without a plugin system: each program that writes text, or a
picture, can become a view.

## Filters operate on the view

The most important rule is that **filters and search operate on the lines of
the view, not on the bytes on disk**. A filter on a JSON key matches the one
line where `jq` puts that key. A filter on an EXIF field matches a field that
only `exiftool` can show. For the decision, see
[ADR 0006](https://github.com/PeteRichardson/recon/blob/main/docs/adr/0006-filters-run-on-the-view.md).

A picture has no lines, so you cannot filter or search it.

## Pictures and previews

The `image` view fills the pane with the picture. recon asks the terminal at
startup how it draws pictures. kitty and Ghostty use the kitty protocol,
iTerm2 uses its own protocol, and other terminals can use sixel. All other
terminals, and recon in tmux, show the picture as half-block characters.

On macOS, the `quicklook` view shows previews of files that a terminal cannot
otherwise show, such as PDFs, Photoshop files, Keynote and Pages documents,
camera RAW files and videos. Thus recon is also a general file browser.

## Syntax color

In a text view, recon colors keywords, strings and comments in approximately
150 languages. By default it uses the palette of your terminal, so it agrees
with your theme. `--theme` selects a bundled theme or a `.tmTheme` file.
Syntax color is a layer on a view, not a view of its own, and filter colors
stay on top of it.
