const sharp = require('sharp');
sharp('app-icon.svg')
  .png()
  .toFile('app-icon-real.png')
  .then(() => console.log('Done'))
  .catch(err => console.error(err));
