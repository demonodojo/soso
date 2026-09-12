/* Host: rtl8168h-2.fw — magic/chksum/fw_start como rtl_fw_format_ok (Linux). */
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#define RTL_VER_SIZE 32
#define FW_OPCODE_SIZE 4

struct fw_info {
	uint32_t magic;
	char version[RTL_VER_SIZE];
	uint32_t fw_start;
	uint32_t fw_len;
	uint8_t chksum;
} __attribute__((packed));

static int rtl_fw_format_ok(const uint8_t *data, size_t size, size_t *out_words)
{
	const struct fw_info *info = (const struct fw_info *)data;
	size_t start, nwords;

	if (size < FW_OPCODE_SIZE)
		return 0;

	if (info->magic == 0) {
		uint8_t checksum = 0;
		size_t i;

		if (size < sizeof(*info))
			return 0;
		for (i = 0; i < size; i++)
			checksum += data[i];
		if (checksum != 0)
			return 0;

		start = info->fw_start;
		if (start > size)
			return 0;
		nwords = info->fw_len;
		if (nwords > (size - start) / FW_OPCODE_SIZE)
			return 0;
		*out_words = nwords;
		return 1;
	}

	if (size % FW_OPCODE_SIZE)
		return 0;
	*out_words = size / FW_OPCODE_SIZE;
	return 1;
}

static int rtl_fw_data_ok(const uint8_t *code, size_t nwords)
{
	size_t index;

	for (index = 0; index < nwords; index++) {
		uint32_t action;
		uint32_t val;
		uint32_t regno;

		memcpy(&action, code + index * FW_OPCODE_SIZE, sizeof(action));
		val = action & 0x0000ffff;
		regno = (action & 0x0fff0000) >> 16;

		switch (action >> 28) {
		case 0x0:
		case 0x1:
		case 0x2:
		case 0x7:
		case 0x8:
		case 0xc:
		case 0xe:
			break;
		case 0x4:
			if (val > 1)
				return 0;
			break;
		case 0x3:
			if (regno > index)
				return 0;
			break;
		case 0x9:
			if (index + 2 >= nwords)
				return 0;
			break;
		case 0xa:
		case 0xb:
		case 0xd:
			if (index + 1 + regno >= nwords)
				return 0;
			break;
		default:
			return 0;
		}
	}
	return 1;
}

/* Linux r8169_phy_config.c:817–823 — rlen TX LPF. */
static uint16_t rlen_pack(uint16_t data)
{
	uint16_t rlen;

	data &= 0x000f;
	rlen = data > 3 ? (uint16_t)(data - 3) : 0;
	return (uint16_t)(rlen | (rlen << 4) | (rlen << 8) | (rlen << 12));
}

/* Linux rtl8168h_2_get_adc_bias_ioffset (r8169_main.c:2241). */
static uint16_t ioffset_pack(uint16_t data1, uint16_t data2)
{
	uint16_t ioffset = (data2 >> 1) & 0x7ff8;

	ioffset |= data2 & 0x0007;
	if (data1 & (1u << 7))
		ioffset |= (uint16_t)(1u << 15);
	return ioffset;
}

int main(int argc, char **argv)
{
	const char *path;
	FILE *f;
	uint8_t *data;
	size_t size, nwords, start;
	const struct fw_info *info;

	if (argc != 2) {
		fprintf(stderr, "uso: %s <rtl8168h-2.fw>\n", argv[0]);
		return 1;
	}
	path = argv[1];
	f = fopen(path, "rb");
	if (!f) {
		perror(path);
		return 1;
	}
	fseek(f, 0, SEEK_END);
	size = (size_t)ftell(f);
	rewind(f);
	data = malloc(size);
	if (!data || fread(data, 1, size, f) != size) {
		fprintf(stderr, "lectura fallida\n");
		return 1;
	}
	fclose(f);

	if (!rtl_fw_format_ok(data, size, &nwords)) {
		fprintf(stderr, "formato inválido\n");
		return 1;
	}

	info = (const struct fw_info *)data;
	start = info->magic == 0 ? info->fw_start : 0;
	if (!rtl_fw_data_ok(data + start, nwords)) {
		fprintf(stderr, "bytecode inválido\n");
		return 1;
	}

	printf("OK: rtl8168h-2.fw magic=%u start=%zu opcodes=%zu\n",
	       info->magic, start, nwords);
	free(data);

	if (rlen_pack(0) != 0 || rlen_pack(3) != 0) {
		fprintf(stderr, "FAIL: rlen nibble≤3 debe ser 0\n");
		return 1;
	}
	if (rlen_pack(4) != 0x1111 || rlen_pack(0x1234) != 0x1111) {
		fprintf(stderr, "FAIL: rlen nibble 4 → 0x1111\n");
		return 1;
	}
	if (rlen_pack(15) != 0xcccc) {
		fprintf(stderr, "FAIL: rlen nibble 15 → 0xcccc\n");
		return 1;
	}
	printf("OK: rlen TX LPF (Linux r8169_phy_config.c:817)\n");

	if (ioffset_pack(0, 0) != 0) {
		fprintf(stderr, "FAIL: ioffset 0,0\n");
		return 1;
	}
	if (ioffset_pack(0, 0x0011) != 0x0009) {
		fprintf(stderr, "FAIL: ioffset data2=0x11 → 0x0009 (got 0x%04x)\n",
			ioffset_pack(0, 0x0011));
		return 1;
	}
	if (ioffset_pack(0x0080, 0) != 0x8000) {
		fprintf(stderr, "FAIL: ioffset data1 bit7 → 0x8000\n");
		return 1;
	}
	if (ioffset_pack(0x0080, 0xffff) != 0xffff) {
		fprintf(stderr, "FAIL: ioffset skip-sentinel 0xffff\n");
		return 1;
	}
	printf("OK: ADC bias ioffset (Linux r8169_main.c:2241)\n");
	return 0;
}
