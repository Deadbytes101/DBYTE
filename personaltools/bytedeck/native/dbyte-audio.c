#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <mmsystem.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define PIPE_NAME "\\\\.\\pipe\\dbyte-audio-v0"
#define TRACK_ALIAS "bytedeck_track"
#define MAX_CMD 8192
#define MAX_REPLY 4096

static int track_open = 0;
static char current_file[MAX_PATH * 4] = {0};
static char current_state[32] = "STOPPED";

static void write_reply(HANDLE pipe, const char *text) {
    DWORD written = 0;
    WriteFile(pipe, text, (DWORD)strlen(text), &written, NULL);
}

static void close_track(void) {
    if (track_open) {
        mciSendStringA("stop " TRACK_ALIAS, NULL, 0, NULL);
        mciSendStringA("close " TRACK_ALIAS, NULL, 0, NULL);
    }
    track_open = 0;
    current_file[0] = '\0';
    strcpy_s(current_state, sizeof(current_state), "STOPPED");
}

static void format_mci_error(char *reply, size_t reply_len, const char *prefix, MCIERROR err) {
    char detail[256] = {0};
    mciGetErrorStringA(err, detail, sizeof(detail));
    if (detail[0] == '\0') {
        snprintf(reply, reply_len, "ERR %s %lu", prefix, (unsigned long)err);
    } else {
        snprintf(reply, reply_len, "ERR %s %lu %s", prefix, (unsigned long)err, detail);
    }
}

static void handle_play(const char *path, char *reply, size_t reply_len) {
    DWORD attrs = GetFileAttributesA(path);
    if (attrs == INVALID_FILE_ATTRIBUTES || (attrs & FILE_ATTRIBUTE_DIRECTORY)) {
        snprintf(reply, reply_len, "ERR FILE_NOT_FOUND");
        return;
    }

    close_track();

    char cmd[MAX_CMD] = {0};
    size_t needed = strlen(path) + strlen("open \"\" alias " TRACK_ALIAS) + 1;
    if (needed > sizeof(cmd)) {
        snprintf(reply, reply_len, "ERR PATH_TOO_LONG");
        return;
    }
    snprintf(cmd, sizeof(cmd), "open \"%s\" alias " TRACK_ALIAS, path);
    MCIERROR err = mciSendStringA(cmd, NULL, 0, NULL);
    if (err != 0) {
        format_mci_error(reply, reply_len, "OPEN_FAILED", err);
        return;
    }

    track_open = 1;
    strncpy_s(current_file, sizeof(current_file), path, _TRUNCATE);

    err = mciSendStringA("play " TRACK_ALIAS, NULL, 0, NULL);
    if (err != 0) {
        close_track();
        format_mci_error(reply, reply_len, "PLAY_FAILED", err);
        return;
    }

    strcpy_s(current_state, sizeof(current_state), "PLAYING");
    snprintf(reply, reply_len, "OK PLAYING %s", current_file);
}

static void handle_pause(char *reply, size_t reply_len) {
    if (!track_open) {
        snprintf(reply, reply_len, "ERR NO_TRACK");
        return;
    }
    MCIERROR err = mciSendStringA("pause " TRACK_ALIAS, NULL, 0, NULL);
    if (err != 0) {
        format_mci_error(reply, reply_len, "PAUSE_FAILED", err);
        return;
    }
    strcpy_s(current_state, sizeof(current_state), "PAUSED");
    snprintf(reply, reply_len, "OK PAUSED");
}

static void handle_resume(char *reply, size_t reply_len) {
    if (!track_open) {
        snprintf(reply, reply_len, "ERR NO_TRACK");
        return;
    }
    MCIERROR err = mciSendStringA("play " TRACK_ALIAS, NULL, 0, NULL);
    if (err != 0) {
        format_mci_error(reply, reply_len, "RESUME_FAILED", err);
        return;
    }
    strcpy_s(current_state, sizeof(current_state), "PLAYING");
    snprintf(reply, reply_len, "OK PLAYING %s", current_file);
}

static void handle_stop(char *reply, size_t reply_len) {
    close_track();
    snprintf(reply, reply_len, "OK STOPPED");
}

static void handle_status(char *reply, size_t reply_len) {
    if (!track_open) {
        snprintf(reply, reply_len, "OK STOPPED");
        return;
    }

    char mode[128] = {0};
    MCIERROR err = mciSendStringA("status " TRACK_ALIAS " mode", mode, sizeof(mode), NULL);
    if (err == 0) {
        if (strcmp(mode, "paused") == 0) {
            strcpy_s(current_state, sizeof(current_state), "PAUSED");
        } else if (strcmp(mode, "playing") == 0) {
            strcpy_s(current_state, sizeof(current_state), "PLAYING");
        } else if (strcmp(mode, "stopped") == 0) {
            strcpy_s(current_state, sizeof(current_state), "STOPPED");
        }
    }

    if (current_file[0] == '\0') {
        snprintf(reply, reply_len, "OK %s", current_state);
    } else {
        snprintf(reply, reply_len, "OK %s %s", current_state, current_file);
    }
}

static void handle_command(const char *command, char *reply, size_t reply_len) {
    if (strncmp(command, "play|", 5) == 0) {
        handle_play(command + 5, reply, reply_len);
    } else if (strcmp(command, "pause") == 0) {
        handle_pause(reply, reply_len);
    } else if (strcmp(command, "resume") == 0) {
        handle_resume(reply, reply_len);
    } else if (strcmp(command, "stop") == 0) {
        handle_stop(reply, reply_len);
    } else if (strcmp(command, "status") == 0) {
        handle_status(reply, reply_len);
    } else if (strcmp(command, "quit") == 0) {
        close_track();
        snprintf(reply, reply_len, "OK BYE");
    } else {
        snprintf(reply, reply_len, "ERR UNKNOWN_COMMAND");
    }
}

static int run_server(void) {
    for (;;) {
        HANDLE pipe = CreateNamedPipeA(
            PIPE_NAME,
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
            1,
            MAX_REPLY,
            MAX_CMD,
            0,
            NULL);
        if (pipe == INVALID_HANDLE_VALUE) {
            fprintf(stderr, "ERR PIPE_CREATE %lu\n", GetLastError());
            return 1;
        }

        BOOL connected = ConnectNamedPipe(pipe, NULL) ? TRUE : (GetLastError() == ERROR_PIPE_CONNECTED);
        if (connected) {
            char command[MAX_CMD] = {0};
            DWORD read = 0;
            if (ReadFile(pipe, command, sizeof(command) - 1, &read, NULL)) {
                command[read] = '\0';
                char reply[MAX_REPLY] = {0};
                handle_command(command, reply, sizeof(reply));
                write_reply(pipe, reply);
                if (strcmp(command, "quit") == 0) {
                    DisconnectNamedPipe(pipe);
                    CloseHandle(pipe);
                    return 0;
                }
            }
        }

        DisconnectNamedPipe(pipe);
        CloseHandle(pipe);
    }
}

static int start_server(void) {
    char exe[MAX_PATH] = {0};
    if (GetModuleFileNameA(NULL, exe, sizeof(exe)) == 0) {
        return 0;
    }

    char command[MAX_PATH + 32] = {0};
    snprintf(command, sizeof(command), "\"%s\" serve", exe);

    STARTUPINFOA si;
    PROCESS_INFORMATION pi;
    ZeroMemory(&si, sizeof(si));
    ZeroMemory(&pi, sizeof(pi));
    si.cb = sizeof(si);

    BOOL ok = CreateProcessA(
        NULL,
        command,
        NULL,
        NULL,
        FALSE,
        CREATE_NO_WINDOW | DETACHED_PROCESS,
        NULL,
        NULL,
        &si,
        &pi);
    if (!ok) {
        return 0;
    }
    CloseHandle(pi.hThread);
    CloseHandle(pi.hProcess);
    return 1;
}

static int connect_pipe(HANDLE *pipe) {
    for (int attempt = 0; attempt < 30; attempt++) {
        *pipe = CreateFileA(
            PIPE_NAME,
            GENERIC_READ | GENERIC_WRITE,
            0,
            NULL,
            OPEN_EXISTING,
            0,
            NULL);
        if (*pipe != INVALID_HANDLE_VALUE) {
            DWORD mode = PIPE_READMODE_MESSAGE;
            SetNamedPipeHandleState(*pipe, &mode, NULL, NULL);
            return 1;
        }
        Sleep(100);
    }
    return 0;
}

static int send_command(const char *command) {
    HANDLE pipe = INVALID_HANDLE_VALUE;
    if (!connect_pipe(&pipe)) {
        if (!start_server()) {
            printf("ERR SERVER_START\n");
            return 1;
        }
        if (!connect_pipe(&pipe)) {
            printf("ERR SERVER_CONNECT\n");
            return 1;
        }
    }

    DWORD written = 0;
    if (!WriteFile(pipe, command, (DWORD)strlen(command), &written, NULL)) {
        CloseHandle(pipe);
        printf("ERR PIPE_WRITE\n");
        return 1;
    }

    char reply[MAX_REPLY] = {0};
    DWORD read = 0;
    if (!ReadFile(pipe, reply, sizeof(reply) - 1, &read, NULL)) {
        CloseHandle(pipe);
        printf("ERR PIPE_READ\n");
        return 1;
    }
    reply[read] = '\0';
    CloseHandle(pipe);

    printf("%s\n", reply);
    return strncmp(reply, "OK ", 3) == 0 ? 0 : 2;
}

static void print_usage(void) {
    printf("usage: dbyte-audio.exe play <file>|pause|resume|stop|status\n");
}

int main(int argc, char **argv) {
    if (argc >= 2 && strcmp(argv[1], "serve") == 0) {
        return run_server();
    }
    if (argc < 2) {
        print_usage();
        return 1;
    }

    if (strcmp(argv[1], "play") == 0) {
        if (argc < 3) {
            printf("ERR FILE_NOT_FOUND\n");
            return 2;
        }
        char command[MAX_CMD] = {0};
        snprintf(command, sizeof(command), "play|%s", argv[2]);
        return send_command(command);
    }
    if (strcmp(argv[1], "pause") == 0) {
        return send_command("pause");
    }
    if (strcmp(argv[1], "resume") == 0) {
        return send_command("resume");
    }
    if (strcmp(argv[1], "stop") == 0) {
        return send_command("stop");
    }
    if (strcmp(argv[1], "status") == 0) {
        return send_command("status");
    }

    printf("ERR UNKNOWN_COMMAND\n");
    return 2;
}
