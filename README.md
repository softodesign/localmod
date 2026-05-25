# LocalMOD

LocalMOD is an open source local AI runtime and desktop app.

It is built for people who want to run, manage, test, and expose AI models from their own computer without depending on a cloud service for everything. It gives you a desktop app for normal use, a local chat system, model management, benchmarking, reference files, downloads, and an OpenAI-compatible API server that other apps can connect to.

The goal is simple:

Run AI locally, manage it cleanly, and make it easy for developers and normal users to use local models without fighting setup hell.

LocalMOD is currently focused on Windows first.

## What LocalMOD Does

LocalMOD is both a desktop app and a local AI runtime.

You can use it as:

- A local AI chat app
- A GGUF model manager
- A local OpenAI-compatible API server
- A model benchmarking tool
- A reference file and context manager
- A simple way to run local AI with llama.cpp bundled inside the installer
- A developer-friendly base for building local AI tools

LocalMOD is designed to feel like a normal app, but still give power users and developers deeper control when they need it.

## Why This Exists

Local AI is powerful, but the user experience is often messy.

People usually have to:

- Download random binaries
- Configure llama.cpp by hand
- Run terminal commands
- Manage model folders manually
- Guess which port a server is using
- Connect different apps with confusing settings
- Deal with crashes when models load
- Figure out which model is fast, slow, good, or broken

LocalMOD tries to make that easier.

The idea is that a user can install one app, add models, chat with them, benchmark them, and turn on an API server for other apps.

Developers can also fork the project and build their own local AI runtime on top of it.

## Main Features

### Local Chat

LocalMOD has a built-in chat page for talking to AI models.

It supports:

- Local GGUF models
- Cloud models
- Streaming replies
- Chat history
- Per-chat model selection
- System prompts
- Thinking mode toggle
- Tool controls
- File and context references
- Smooth chat switching
- Regenerate, edit, copy, and delete messages

The chat UI is designed to feel responsive even while local models are loading.

### Local GGUF Model Support

LocalMOD supports local GGUF models through llama.cpp.

The app can load local GGUF files and run chat through the bundled runtime.

### Bundled llama.cpp Runtime

LocalMOD is set up to ship with llama.cpp inside the installer.

For normal users, the goal is:

1. Download the LocalMOD installer.
2. Install the app.
3. Use local AI.
4. No separate llama.cpp download.

The bundled runtime lives inside:

```text
src-tauri/binaries/llama-runtime/
```

Before shipping a release, this folder must contain the real runtime files, such as:

```text
llama-server.exe
llama.dll
llama-common.dll
ggml*.dll
required OpenMP or backend DLLs
```

The installer bundles this folder as an app resource. Users do not need to see or touch it.

### API Server

LocalMOD includes an OpenAI-compatible API server.

This lets other apps connect to LocalMOD using normal OpenAI-style endpoints:

```text
GET  /v1/models
POST /v1/chat/completions
```

The Settings page has one simple API Server section for users.

Users can:

- Turn the API server on
- Turn it off
- Copy the API URL
- Choose who can connect
- Set a port
- Enable or disable API key auth

The API server is powered by the standalone `localmod-server` binary.

That means it can keep running even after the desktop window is closed.

### Headless Server

LocalMOD also builds a headless server binary:

```text
localmod-server.exe
```

This can run without the desktop app.

It is useful for:

- VPS hosting
- LAN servers
- backend services
- developer tools
- automation
- running LocalMOD as a local AI API only

Example:

```powershell
localmod-server.exe --host 0.0.0.0 --port 11435 --data-dir D:\LocalMOD --auth bearer --api-key your-secret-key
```

Then another app can connect to:

```text
http://SERVER_IP:11435/v1
```

With Bearer auth:

```text
Authorization: Bearer your-secret-key
```

### Model Benchmarking

LocalMOD includes a Benchmark tab.

You can test one model or compare two models.

The benchmark checks things like:

- Latency
- Speed
- Basic reasoning
- Coding output
- Estimated throughput
- Memory changes

This helps users understand which models are actually useful on their machine.

### Model Management

LocalMOD includes a Models section for managing models.

It supports:

- Local model registration
- Cloud model entries
- Model metadata
- Model loading
- Model deletion
- Model discovery and download flows

The app is built around the idea that users should not have to manually remember where every file lives.

### Downloads

LocalMOD includes a Downloads area for model downloads.

The backend has a download manager that supports long-running downloads, pause, resume, cancel, and dismiss behavior.

### Reference Files

LocalMOD has a Reference section for adding files and text that can be used in chat.

Users can reference documents in chat and include context in model prompts.

This is useful for:

- notes
- docs
- code snippets
- research files
- project knowledge

### Agent Mode

LocalMOD has an agent mode for local tool use.

When enabled, the AI can use tools such as:

- run commands
- read files
- write files
- edit files
- create folders
- install packages
- debug
- gather system info

These features are meant for local developer workflows.

Use them carefully.

### Cloud Models

LocalMOD can also connect to cloud AI providers.

The app currently has support paths for providers like:

- OpenAI
- Anthropic
- OpenRouter
- Custom providers

Cloud models are useful when a user wants speed, large models, or remote providers while still using LocalMOD as the main interface.

### Local Image Generation

Local image generation was removed from the app.

Only cloud image generation is supported.

This keeps the local runtime focused on chat and text models.

## Tech Stack

LocalMOD uses:

- Tauri 2
- Rust
- Svelte 5
- SvelteKit
- TypeScript
- Tailwind CSS
- SQLite
- Axum for the API server
- llama.cpp through bundled `llama-server`

The desktop app is Tauri.

The frontend is Svelte.

The backend is Rust.

The local API server is also Rust.

## Project Structure

Important folders:

```text
src/
  Frontend Svelte app

src/routes/
  App pages like Chats, Models, Settings, Benchmark, Downloads

src/lib/
  Frontend helpers, components, and Tauri bridge code

src-tauri/
  Rust backend and Tauri app

src-tauri/src/
  Rust source code

src-tauri/src/bin/localmod-server.rs
  Headless API server binary

src-tauri/binaries/
  Runtime binaries bundled into the app installer

src-tauri/binaries/llama-runtime/
  llama.cpp runtime files for local model inference

scripts/
  Build and packaging helper scripts
```

## Requirements For Development

You need:

- Node.js
- npm
- Rust
- Cargo
- Tauri CLI through npm scripts
- Windows build tools if building on Windows

Recommended:

- Recent Windows 10 or Windows 11
- A modern CPU
- Enough RAM for the models you want to run
- GPU support if your llama.cpp runtime build supports it

## Install Dependencies

Clone the repo:

```bash
git clone https://github.com/softodesign/localmod.git
cd localmod
```

Install frontend dependencies:

```bash
npm install
```

Rust dependencies are handled by Cargo.

## Run The Desktop App In Development

Run:

```bash
npm run tauri dev
```

This starts the Svelte dev server and the Tauri desktop app.

If you want to run the normal llama sidecar build:

```bash
npm run tauri:llama
```

## Run Frontend Only

```bash
npm run dev
```

This is useful for frontend work, but Tauri backend commands will not work the same way as the full desktop app.

## Run Checks

Frontend and Svelte checks:

```bash
npm run check
```

Rust checks:

```bash
cd src-tauri
cargo check
```

Check the headless server binary:

```bash
cd src-tauri
cargo check --bin localmod-server
```

## Run The Headless API Server

From the Rust project:

```bash
cd src-tauri
cargo run --bin localmod-server -- --host 127.0.0.1 --port 11435 --data-dir ../localmod-data
```

With API key auth:

```bash
cargo run --bin localmod-server -- --host 0.0.0.0 --port 11435 --data-dir D:\LocalMOD --auth bearer --api-key your-secret-key
```

Then test:

```bash
curl http://127.0.0.1:11435/v1/models
```

Chat completions use:

```text
POST /v1/chat/completions
```

Example body:

```json
{
  "model": "Your Model Name",
  "messages": [
    {
      "role": "user",
      "content": "Hello"
    }
  ],
  "stream": false
}
```

## Build The App

Build frontend:

```bash
npm run build
```

Build the full desktop installer:

```bash
npm run build:llama
```

This runs:

```bash
npm run prepare:bundle
tauri build
```

The prepare step builds and copies:

```text
localmod-server.exe
```

into:

```text
src-tauri/binaries/
```

Then Tauri bundles it into the installer.

## Installer Output

After a successful Windows build, the installer files are created under:

```text
src-tauri/target/release/bundle/
```

Common outputs:

```text
src-tauri/target/release/bundle/nsis/LocalModFiles_0.1.0_x64-setup.exe
src-tauri/target/release/bundle/msi/LocalModFiles_0.1.0_x64_en-US.msi
```

For normal users, the NSIS setup exe is the easiest file to ship.

## Shipping To Users

The goal is to ship one installer:

```text
LocalModFiles_0.1.0_x64-setup.exe
```

The user runs it and gets:

- LocalMOD desktop app
- headless API server binary
- bundled llama.cpp runtime
- frontend assets
- Rust backend
- local app data setup

They should not need to manually download llama.cpp.

Before public release, test the installer on a clean Windows machine.

## Important Release Checklist

Before shipping a public build:

- Make sure `src-tauri/binaries/llama-runtime/` contains real llama.cpp files.
- Make sure `llama-server.exe` exists in that folder.
- Make sure required DLLs exist in that folder.
- Run `npm run build:llama`.
- Install the generated setup exe on a clean machine.
- Open LocalMOD.
- Add or download a GGUF model.
- Load a model.
- Send a chat message.
- Start the API Server from Settings.
- Call `/v1/models`.
- Call `/v1/chat/completions`.
- Close the desktop window and confirm the API server behavior you expect.
- Test uninstall.
- Test reinstall.
- Sign the installer if you want fewer Windows security warnings.

## About The Local Runtime

LocalMOD uses a bundled `llama-server` runtime by default.

This is intentionally separate from the main desktop exe.

That makes the app easier to build and avoids forcing every developer to compile llama.cpp in-process.

There is also an optional `llama-engine` feature for in-process llama.cpp work, but the default path is the bundled server runtime.

For now, the bundled server path is the recommended production path.

## API Server For Other Apps

When the API Server is on, other apps can connect using the LocalMOD API URL.

Example:

```text
http://127.0.0.1:11435/v1
```

If the user chooses:

```text
127.0.0.1
```

only the same computer can connect.

If the user chooses:

```text
0.0.0.0
```

other devices on the network can connect if the firewall allows it.

For server hosting, use a proper firewall, API key, and reverse proxy.

## Hosting On A VPS

You can run only the headless server on a VPS.

Copy these to the server:

- `localmod-server`
- `llama-runtime/`
- your model files
- a data directory

Run:

```bash
./localmod-server \
  --host 0.0.0.0 \
  --port 11435 \
  --data-dir /opt/localmod/data \
  --runtime-dir /opt/localmod/llama-runtime \
  --auth bearer \
  --api-key your-secret-key
```

Then point apps to:

```text
http://your-server-ip:11435/v1
```

For production, put it behind Caddy, Nginx, or another reverse proxy with HTTPS.

## Contributing

Contributions are welcome.

You can help with:

- UI improvements
- bug fixes
- model loading reliability
- server hosting features
- benchmark improvements
- docs
- installer testing
- Windows packaging
- Linux and macOS support
- better model download flows
- better local runtime management
- accessibility
- performance

## How To Fork And Work On It

1. Fork the repo on GitHub.
2. Clone your fork.
3. Install dependencies with `npm install`.
4. Run the app with `npm run tauri dev`.
5. Make your changes.
6. Run checks.
7. Commit your changes.
8. Open a pull request.

Example:

```bash
git clone https://github.com/softodesign/localmod.git
cd localmod
npm install
npm run tauri dev
```

Before opening a pull request:

```bash
npm run check
cd src-tauri
cargo check
```

If you touched the headless server:

```bash
cargo check --bin localmod-server
```

If you touched packaging:

```bash
npm run build:llama
```

## Current Status

LocalMOD is actively being built.

Some areas are stable enough to use, while others are still evolving.

The project is not finished.

Expect changes.

If you want to use it seriously, test your workflow and report issues.

## FAQ

### Is LocalMOD fully offline?

Local GGUF chat can run locally.

Cloud models and cloud image generation need internet access.

### Does the user need to download llama.cpp?

No, not if you ship the installer with the bundled llama runtime folder filled correctly.

### Is it one portable exe?

No.

It is one installer exe that installs the app and bundled runtime files.

That is more reliable for Windows because llama.cpp uses DLLs.

### Can LocalMOD run as a server?

Yes.

Use `localmod-server`.

The desktop Settings page can also start and stop the API server.

### Can I use it with OpenAI-compatible clients?

Yes.

Use:

```text
/v1/models
/v1/chat/completions
```

### Can I add my own provider or model source?

Yes.

Fork the repo and add the provider in the Rust and frontend model paths.

## License

This project is currently marked as MIT in `package.json`.

If you publish the repo, make sure the final `LICENSE` file matches the license you want.

## Contact

For questions, support, or collaboration:

```text
support@softodesign.com
```

Made by al wassikhan.

Powered by softodesign.