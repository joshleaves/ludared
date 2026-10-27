#include <stdio.h>
#include <stdlib.h>
#include <libvcd/sector.h>

enum { RECORD_SIZE = 4 + 2324, RAW_SECTOR_SIZE = 2352 };

int main(int argc, char **argv) {
  FILE *input;
  FILE *output;
  unsigned char record[RECORD_SIZE];
  unsigned char sector[RAW_SECTOR_SIZE];
  unsigned extent = 0;

  if (argc != 3) {
    fprintf(stderr, "usage: %s manifest raw-output\n", argv[0]);
    return 2;
  }
  input = fopen(argv[1], "rb");
  output = fopen(argv[2], "wb");
  if (input == NULL || output == NULL) {
    perror("open");
    return 1;
  }
  while (fread(record, 1, RECORD_SIZE, input) == RECORD_SIZE) {
    _vcd_make_mode2(sector, record + 4, extent++, record[0], record[1], record[2], record[3]);
    if (fwrite(sector, 1, RAW_SECTOR_SIZE, output) != RAW_SECTOR_SIZE) {
      perror("write");
      return 1;
    }
  }
  if (ferror(input) || extent == 0) {
    fprintf(stderr, "invalid or empty sector manifest\n");
    return 1;
  }
  fclose(input);
  fclose(output);
  return 0;
}
