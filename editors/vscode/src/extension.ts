// VS Code client for smali-lsp: starts the server over stdio and forwards the `smali` settings.
import * as vscode from "vscode";
import { LanguageClient, LanguageClientOptions, ServerOptions } from "vscode-languageclient/node";

let client: LanguageClient | undefined;

function serverSettings(): Record<string, unknown> {
  const c = vscode.workspace.getConfiguration("smali");
  const opt = (s: string) => (s ? s : undefined);
  return {
    androidJar: opt(c.get<string>("androidJar", "")),
    apiLevel: c.get<number | null>("apiLevel", null) ?? undefined,
    extraRoots: c.get<string[]>("extraRoots", []),
    cacheDir: opt(c.get<string>("cacheDir", "")),
    diagnostics: c.get<Record<string, string>>("diagnostics", {}),
  };
}

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  const command = vscode.workspace.getConfiguration("smali").get<string>("serverPath", "") || "smali-lsp";
  const serverOptions: ServerOptions = { command, args: [] };
  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: "file", language: "smali" }],
    initializationOptions: { smali: serverSettings() },
    synchronize: { fileEvents: vscode.workspace.createFileSystemWatcher("**/*.smali") },
  };
  client = new LanguageClient("smali-lsp", "Smali", serverOptions, clientOptions);
  context.subscriptions.push(
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (e.affectsConfiguration("smali") && client) {
        void client.sendNotification("workspace/didChangeConfiguration", { settings: { smali: serverSettings() } });
      }
    }),
  );
  await client.start();
}

export async function deactivate(): Promise<void> {
  await client?.stop();
}
