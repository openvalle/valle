export const composition = { width: 320, height: 180, fps: 1, duration: 1 };
export default function Scene() {
  return (
    <View style={{ width: 320, height: 180, backgroundColor: "white" }}>
      <svg width="320" height="180">
        <defs>
          <marker
            id="default"
            markerWidth="20"
            markerHeight="20"
            viewBox="0 0 10 10"
            refX="5"
            refY="5"
            markerUnits="userSpaceOnUse"
          >
            <rect x="-5" y="-5" width="20" height="20" fill="#2878b8" />
          </marker>
          <marker
            id="visible"
            markerWidth="20"
            markerHeight="20"
            viewBox="0 0 10 10"
            refX="5"
            refY="5"
            markerUnits="userSpaceOnUse"
            overflow="visible"
          >
            <rect x="-5" y="-5" width="20" height="20" fill="#2878b8" />
          </marker>
          <marker
            id="css"
            markerWidth="10"
            markerHeight="10"
            viewBox="0 0 10 10"
            refX="5"
            refY="5"
            overflow="hidden"
            style={{ overflow: "visible" }}
          >
            <rect x="-5" y="-5" width="20" height="20" fill="#d06030" />
          </marker>
          <marker
            id="hidden"
            markerWidth="10"
            markerHeight="10"
            viewBox="0 0 10 10"
            refX="5"
            refY="5"
            overflow="visible"
            style={{ overflow: "hidden" }}
          >
            <rect x="-5" y="-5" width="20" height="20" fill="#d06030" />
          </marker>
        </defs>
        <path d="M20 40H110" stroke="#222" strokeWidth="2" markerEnd="url(#default)" />
        <path d="M180 40H270" stroke="#222" strokeWidth="2" markerEnd="url(#visible)" />
        <path d="M110 90V140" stroke="#222" strokeWidth="2" markerEnd="url(#css)" />
        <path d="M270 90V140" stroke="#222" strokeWidth="2" markerEnd="url(#hidden)" />
      </svg>
    </View>
  );
}
