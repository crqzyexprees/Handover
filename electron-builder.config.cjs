/** @type {import('electron-builder').Configuration} */
module.exports = {
  appId: 'com.handover.app',
  productName: 'Handover',
  directories: {
    output: 'release',
  },
  files: ['dist/**/*', 'src/main.cjs', 'src/preload.cjs'],
  extraResources: [
    {
      from: 'backend/dist/handover-backend',
      to: 'backend/handover-backend',
    },
  ],
  linux: {
    // AppImage (portable) + rpm (native Fedora package). The rpm/deb targets
    // use electron-builder's bundled fpm, whose Ruby needs libcrypt.so.1 (not
    // present on modern Fedora by default). The build is run with
    // LD_LIBRARY_PATH pointing at a libcrypt.so.1 shim so fpm can run; rpmbuild
    // itself is already installed. (deb omitted; add it back the same way.)
    target: ['AppImage', 'rpm'],
    maintainer: 'crqzyexprees <machinelearningpennstate@gmail.com>',
    category: 'Development',
  },
}
