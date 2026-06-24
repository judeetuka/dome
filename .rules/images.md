# Image and foundation rules

Dome uses native `.dome` files as its canonical artifact format.

Canonical extensions:

- `.foundation.dome`: bootable immutable base OS foundation
- `.seal.dome`: saved chamber state
- `.cell.toml`: TOML blueprint/configuration

Dome must not use tar, zip, qcow2, VHDX, or raw disks as the canonical user-facing format. Platform engines may prepare temporary engine-specific disk formats internally, but those are cache artifacts, not the portable format.

A `.dome` file should be a custom binary container with:

- magic header and format version
- encrypted manifest
- authenticated chunk table
- encrypted chunks
- integrity metadata
- signature metadata
- key-wrapping metadata

The format may be custom. The cryptography must not be custom. Use reviewed encryption and authentication primitives.
