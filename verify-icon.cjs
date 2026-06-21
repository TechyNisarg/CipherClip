const sharp = require('sharp');
sharp('app-icon-real.png')
  .stats()
  .then(stats => {
    console.log('Image stats:');
    stats.channels.forEach((ch, i) => {
      console.log(`  Channel ${i}: min=${ch.min}, max=${ch.max}, mean=${ch.mean.toFixed(2)}`);
    });
    console.log('  isOpaque:', stats.isOpaque);
    console.log('Image has visible content:', stats.channels.some(ch => ch.min !== ch.max));
  })
  .catch(err => console.error(err));
