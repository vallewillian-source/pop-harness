#!/usr/bin/env node

import { createRequire } from "node:module";
import path from "node:path";
import fs from "node:fs";
import readline from "node:readline";

const require = createRequire(import.meta.url);

function fail(message) {
    process.stdout.write(JSON.stringify({ ok: false, error: message }) + "\n");
    process.exit(2);
}

const rootIndex = process.argv.indexOf("--root");
if (rootIndex < 0 || !process.argv[rootIndex + 1]) {
    fail("rename_ts: --root is required");
}
const root = path.resolve(process.argv[rootIndex + 1]);
const consoleDir = path.join(root, "console");
const typescriptPath = path.join(consoleDir, "node_modules", "typescript");
if (!fs.existsSync(typescriptPath)) {
    fail(`rename_ts: TypeScript is not installed at ${typescriptPath}`);
}
const ts = require(typescriptPath);

const configPath = ts.findConfigFile(consoleDir, ts.sys.fileExists, "tsconfig.json");
if (!configPath) {
    fail(`rename_ts: no tsconfig.json under ${consoleDir}`);
}
const config = ts.readConfigFile(configPath, ts.sys.readFile);
if (config.error) {
    fail(`rename_ts: tsconfig.json: ${ts.flattenDiagnosticMessageText(config.error.messageText, " ")}`);
}
const parsed = ts.parseJsonConfigFileContent(config.config, ts.sys, consoleDir);
if (parsed.errors.length) {
    fail(`rename_ts: tsconfig.json: ${ts.flattenDiagnosticMessageText(parsed.errors[0].messageText, " ")}`);
}

const versions = new Map();
const host = {
    getScriptFileNames: () => parsed.fileNames,
    getScriptVersion: (file) => versions.get(file) || "0",
    getScriptSnapshot: (file) => {
        const text = ts.sys.readFile(file);
        return text === undefined ? undefined : ts.ScriptSnapshot.fromString(text);
    },
    getCurrentDirectory: () => consoleDir,
    getCompilationSettings: () => parsed.options,
    getDefaultLibFileName: (options) => ts.getDefaultLibFilePath(options),
    fileExists: ts.sys.fileExists,
    readFile: ts.sys.readFile,
    readDirectory: ts.sys.readDirectory,
    directoryExists: ts.sys.directoryExists,
    getDirectories: ts.sys.getDirectories,
    useCaseSensitiveFileNames: () => ts.sys.useCaseSensitiveFileNames,
};

const service = ts.createLanguageService(host);

function positionOf(request) {
    const source = service.getProgram().getSourceFile(request.file);
    if (!source) {
        return -1;
    }
    return source.getPositionOfLineAndCharacter(request.line - 1, request.character);
}

function groupChanges(fileChanges) {
    const changes = [];
    for (const entry of fileChanges) {
        for (const change of entry.textChanges) {
            changes.push({
                file: path.resolve(entry.fileName),
                start: change.span.start,
                length: change.span.length,
                newText: change.newText,
            });
        }
    }
    return changes;
}

function handle(request) {
    if (request.op === "prepare") {
        const position = positionOf(request);
        if (position < 0) {
            return { ok: false, error: `file not in the program: ${request.file}` };
        }
        const info = service.getRenameInfo(request.file, position, { allowRenameOfImportPath: false });
        return { ok: true, canRename: Boolean(info.canRename), reason: info.localizedErrorMessage || "" };
    }
    if (request.op === "rename") {
        const position = positionOf(request);
        if (position < 0) {
            return { ok: false, error: `file not in the program: ${request.file}` };
        }
        const locations = service.findRenameLocations(request.file, position, false, false, {
            providePrefixAndSuffixTextForRename: false,
        });
        if (!locations || !locations.length) {
            return { ok: true, changes: [] };
        }
        const byFile = new Map();
        for (const location of locations) {
            const list = byFile.get(location.fileName) || [];
            list.push({ span: location.textSpan, newText: request.newName });
            byFile.set(location.fileName, list);
        }
        const fileChanges = [...byFile.entries()].map(([fileName, textChanges]) => ({ fileName, textChanges }));
        return { ok: true, changes: groupChanges(fileChanges) };
    }
    if (request.op === "fileRename") {
        const edits = service.getEditsForFileRename(request.oldFile, request.newFile, {}, {});
        return { ok: true, changes: groupChanges(edits) };
    }
    if (request.op === "ping") {
        return { ok: true, files: parsed.fileNames.length };
    }
    return { ok: false, error: `unknown op: ${request.op}` };
}

const input = readline.createInterface({ input: process.stdin });
input.on("line", (line) => {
    if (!line.trim()) {
        return;
    }
    let request;
    try {
        request = JSON.parse(line);
    } catch (error) {
        process.stdout.write(JSON.stringify({ ok: false, error: `invalid JSON: ${error.message}` }) + "\n");
        return;
    }
    if (request.op === "quit") {
        process.stdout.write(JSON.stringify({ ok: true }) + "\n");
        process.exit(0);
    }
    try {
        process.stdout.write(JSON.stringify(handle(request)) + "\n");
    } catch (error) {
        process.stdout.write(JSON.stringify({ ok: false, error: String(error && error.message ? error.message : error) }) + "\n");
    }
});
