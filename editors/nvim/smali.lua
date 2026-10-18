-- Neovim setup for smali-lsp (0.11+ uses vim.lsp.config; older versions fall back to vim.lsp.start).
-- Put in your config, e.g. ~/.config/nvim/after/plugin/smali.lua. Highlighting: `:TSInstall smali`.
vim.filetype.add({ extension = { smali = "smali" } })

local settings = {
  smali = {
    -- androidJar = "/path/to/android.jar",
    -- extraRoots = { "/path/to/framework-smali" },
    -- diagnostics = { S301 = "off" },
  },
}

if vim.lsp.config then
  vim.lsp.config("smali_lsp", {
    cmd = { "smali-lsp" },
    filetypes = { "smali" },
    root_markers = { "apktool.yml", ".git" },
    init_options = settings,
  })
  vim.lsp.enable("smali_lsp")
else
  vim.api.nvim_create_autocmd("FileType", {
    pattern = "smali",
    callback = function(args)
      local root = vim.fs.dirname(vim.fs.find({ "apktool.yml", ".git" }, { upward = true, path = vim.api.nvim_buf_get_name(args.buf) })[1])
      vim.lsp.start({ name = "smali_lsp", cmd = { "smali-lsp" }, root_dir = root, init_options = settings })
    end,
  })
end
