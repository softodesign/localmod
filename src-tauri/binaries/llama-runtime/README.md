This folder is bundled into the LocalMOD installer.

Before shipping a release, it must contain the real llama.cpp runtime files:

- llama-server.exe
- llama.dll
- llama-common.dll
- ggml*.dll
- any required OpenMP / backend DLLs

End users should not need to download these files separately. They are installed as hidden app resources and used by LocalMOD automatically.
