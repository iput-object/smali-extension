# Vendored tree-sitter-smali

Copy of [tree-sitter-grammars/tree-sitter-smali](https://github.com/tree-sitter-grammars/tree-sitter-smali)
at commit `fdfa6a1febc43c7467aa7e937b87b607956f2346` (MIT, see `LICENSE`; authors Yotam Nachum and
Amaan Qureshi). Kept here instead of a fork so fixes land with the code that needs them.
Upstream tests are in `test/corpus`; our additions are in `test/corpus/vendored_fixes`.

Regenerate after editing `grammar.js` (ABI 14 for the tree-sitter 0.20 runtime):

    npx tree-sitter-cli@0.25.10 generate --abi 14
    npx tree-sitter-cli@0.25.10 test

## Changes from upstream

Each fix came from the corpus runner (`smali-check --no-parse-errors`) and is checked against
google/smali `smaliParser.g`:

1. `.param pN, "name"` may be followed by annotations and `.end param`; `.end param` is now its own
   directive (`end_param_directive`). smaliParser.g `parameter_directive` attaches annotations to
   the parameter only when `.end param` follows.
2. Field and method names that are also keywords (`final`, `annotation`, `synchronized`,
   `volatile`, `constructor`, `public`, `blacklist`, ...). In declarations the name is lexed with
   its trailing `:` or `(`, so the longest match beats the keyword. The `field_identifier` node in
   a `.field` line therefore includes the `:`.
3. `Infinityf` and `-Infinityf` float literals.
4. Annotation values may be any type, including primitives (`factory = V`).
5. `.super` is optional (`Ljava/lang/Object;` has none).

Worth sending upstream as PRs.
