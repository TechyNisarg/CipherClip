const sharp = require("sharp");
async function main() {
  const image = sharp("C:/Users/Nisarg Prajapati/.gemini/antigravity/brain/797202c8-356e-4dfd-8357-0b24579bd062/media__1781525065273.png");
  const metadata = await image.metadata();
  const size = Math.min(metadata.width, metadata.height);
  const buffer = await image
    .extract({
      left: Math.floor((metadata.width - size) / 2),
      top: Math.floor((metadata.height - size) / 2),
      width: size,
      height: size
    })
    .toBuffer();
  await sharp(buffer).toFile("app-icon.png");
  await sharp(buffer).toFile("public/logo.png");
  console.log("Done!");
}
main();
