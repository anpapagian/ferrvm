# TODO

Next steps:
- [x] Add support for virtio-block.
- [ ] Add support for virtio-net.
- [ ] Emulate MSR in bhyve and handle rdmsr/wrmsr exits.
- [ ] Add support for multiple CPUs in the guest.
- [ ] Handle poweroff properly.
- [ ] Add a PCI bus and a virtio-pci transport (and remove `pci=off`).
- [ ] Cross-build ferrvm in Linux for illumos to run bhyve tests in CI.
- [ ] Add more tests beyond memory and bus.
- [ ] Fix setting memory size over ~3.5GB as this will collide with virtio.
