/* Minimal init for the Dome KVM boot test.
 *
 * Deliberately avoids stdio and buffering: everything goes through raw write() to an fd
 * decided at startup, so a missing console shows up as missing output rather than as
 * silently buffered output that never flushes.
 */

#include <fcntl.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/reboot.h>
#include <sys/stat.h>
#include <sys/io.h>
#include <sys/sysmacros.h>
#include <unistd.h>

static int out_fd = 1;

static void emit(const char *text) {
    write(out_fd, text, strlen(text));
}

static void emit_line(const char *text) {
    emit(text);
    emit("\n");
}

/* Make sure fd 1 is a real console. devtmpfs is usually already mounted by the kernel,
 * but fall back to creating the node by hand since init runs as uid 0 in the guest. */
static void attach_console(void) {
    emit_line("dome-init: start");

    int console = open("/dev/console", O_RDWR);
    if (console < 0) {
        mkdir("/dev", 0755);
        mount("devtmpfs", "/dev", "devtmpfs", 0, NULL);
        mknod("/dev/console", S_IFCHR | 0600, makedev(5, 1));
        console = open("/dev/console", O_RDWR);
    }

    if (console >= 0) {
        dup2(console, STDIN_FILENO);
        dup2(console, STDOUT_FILENO);
        dup2(console, STDERR_FILENO);
        out_fd = (console > STDERR_FILENO) ? STDOUT_FILENO : console;
        if (console > STDERR_FILENO) {
            close(console);
        }
        emit_line("dome-init: console attached");
    } else {
        emit_line("dome-init: console attach FAILED, writing to fd 1");
    }
}

/* Write straight to COM1 through the IO ports.
 *
 * This exists because userspace writes to /dev/console are interrupt driven: the 8250
 * driver queues bytes and drains them in its TX interrupt handler. printk survives
 * without that (it polls the port), which is why kernel messages appear while a
 * userspace write appears to do nothing. Driving the port directly sidesteps the tty
 * layer and proves the IO exit path works from userspace too. */
static void direct_serial(const char *text) {
    if (ioperm(0x3F8, 8, 1) != 0) {
        return;
    }

    for (const char *cursor = text; *cursor != '\0'; cursor++) {
        if (*cursor == '\n') {
            outb('\r', 0x3F8);
        }
        outb((unsigned char)*cursor, 0x3F8);
    }
}

int main(void) {
    char buffer[64];

    emit_line("dome-init: hello from userspace");

    attach_console();
    emit_line("dome-init: initramfs init reached");

    mkdir("/proc", 0755);
    mkdir("/sys", 0755);

    if (mount("proc", "/proc", "proc", 0, NULL) == 0) {
        emit_line("dome-init: /proc mounted");
    } else {
        emit_line("dome-init: /proc mount failed");
    }

    int version = open("/proc/version", O_RDONLY);
    if (version >= 0) {
        ssize_t count = read(version, buffer, sizeof(buffer) - 1);
        close(version);
        if (count > 0) {
            buffer[count] = '\0';
            char *newline = strchr(buffer, '\n');
            if (newline != NULL) {
                *newline = '\0';
            }
            emit("dome-init: ");
            emit_line(buffer);
        }
    }

    direct_serial("dome-init: direct serial write from userspace\n");
    emit_line("dome-init: boot complete");
    sync();
    reboot(RB_POWER_OFF);

    for (;;) {
        pause();
    }
}
