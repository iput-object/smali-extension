# Credits

- [tree-sitter-smali](https://github.com/tree-sitter-grammars/tree-sitter-smali) by Yotam Nachum,
  Amaan Qureshi and contributors (MIT). Vendored with fixes in `grammar/tree-sitter-smali`
  (see its `VENDORED.md`); its queries are the base of the editor highlight queries.
- [google/smali](https://github.com/google/smali) (BSD-3-Clause), originally by Ben Gruver
  (JesusFreke). The opcode table is generated from its `dexlib2` `Opcode.java`; its lexer and
  parser define the syntax rules the diagnostics follow.
- [nvim-treesitter](https://github.com/nvim-treesitter/nvim-treesitter) (Apache-2.0) smali queries
  informed the editor queries.
