export prefix ?= /usr
sysconfdir ?= /etc
bindir = $(prefix)/bin
libdir = $(prefix)/lib

BINARY = system76-scheduler
ID = com.system76.Scheduler
TARGET = debug
DEBUG ?= 0

.PHONY = all clean install uninstall vendor

ifeq ($(DEBUG),0)
	TARGET = release
	ARGS += --release
endif

VENDOR ?= 0
ifneq ($(VENDOR),0)
	ARGS += --frozen
endif

TARGET_BIN="$(DESTDIR)$(bindir)/$(BINARY)"

all: extract-vendor
	cargo build $(ARGS)

clean:
	cargo clean

distclean:
	rm -rf .cargo vendor vendor.tar target

vendor:
	mkdir -p .cargo
	cargo vendor | head -n -1 > .cargo/config
	echo 'directory = "vendor"' >> .cargo/config
	tar pcf vendor.tar vendor
	rm -rf vendor

extract-vendor:
ifeq ($(VENDOR),1)
	rm -rf vendor; tar pxf vendor.tar
endif

install:
	mkdir -p $(DESTDIR)$(sysconfdir)/system76-scheduler/assignments
	install -Dm0644 "data/config.ron" "$(DESTDIR)$(sysconfdir)/system76-scheduler/config.ron"
	install -Dm0644 "data/auto.ron" "$(DESTDIR)$(sysconfdir)/system76-scheduler/assignments/default.ron"
	install -Dm04755 "target/$(TARGET)/$(BINARY)" "$(TARGET_BIN)"
	install -Dm0644 "data/$(ID).service" "$(DESTDIR)$(libdir)/systemd/system/$(ID).service"
	install -Dm0644 "data/$(ID).conf" "$(DESTDIR)$(sysconfdir)/dbus-1/system.d/$(ID).conf"


uninstall:
	rm "$(TARGET_BIN)"