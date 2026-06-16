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
      from: '../handover_rust/target/release/handover-backend',
      to: 'backend/handover-backend',
    },
  ],
  linux: {
    target: ['AppImage'],
    maintainer: 'crqzyexprees <machinelearningpennstate@gmail.com>',
    category: 'Development',
  },
}
