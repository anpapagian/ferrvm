TARGET   = bhyve_api
SRC      = bhyve_api.c

.PHONY: all clean

all: $(TARGET)

$(TARGET): $(SRC)
	$(CC) -m64 $(CFLAGS) -o $@ $<

clean:
	rm -f $(TARGET)
