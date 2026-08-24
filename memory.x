MEMORY
{
  /* nRF52840 with the Adafruit UF2 bootloader used by the XIAO BLE. */
  /* Keep firmware below RMK storage at 0xA0000 and device settings at 0xA6000. */
  FLASH : ORIGIN = 0x00001000, LENGTH = 636K
  RAM : ORIGIN = 0x20000008, LENGTH = 255K
}
