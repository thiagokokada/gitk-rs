# Diff controls

The diff toolbar follows gitk's order and labels: Diff / Old version / New
version, Lines of context, Ignore space change, and the word display menu.
Options apply to commits and local changes and persist within the window.

Old version hides additions; New version hides removals. These show the chosen
side of the displayed hunks, rather than loading the complete file. Context
starts at three lines and accepts zero through ten million lines. Ignore space
change uses `git diff -w`, matching gitk (including ignoring all whitespace).

Line diff retains syntax highlighting. Markup words and Color words use Git's
porcelain word diff, joining unchanged and changed fragments into display lines.
Markup adds `[-removed-]` and `{+added+}` delimiters; both modes color changed
fragments. Word colors take priority over syntax colors. Copying reads the
rendered document, including markup when selected; Copy without diff markers
removes the leading line marker.

Patch generation, filtering and word preparation run on the diff worker.
Changing options cancels the previous request and advances the selection token.
The line list remains virtualized, and file navigation uses the filtered
document's line offsets.
