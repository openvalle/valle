const latitude = 31.2;
const mercatorY = Math.log(Math.tan(Math.PI / 4 + latitude * Math.PI / 360));

export default function HostMathMap() {
  return <View key="point" style={{ position: "absolute", left: 100, top: mercatorY, width: 8, height: 8 }} />;
}
