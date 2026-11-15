export const composition = { width: 320, height: 180, fps: 1, duration: 1 };
export default function Scene() {
  return (
    <svg xmlns="http://www.w3.org/2000/svg" width="320" height="180" fontSize="24" fill="#182433">
      <rect width="320" height="180" fill="white" />
      <g xmlSpace="preserve">
        <text x="16" y="40">
          {" A   B "}
        </text>
        <text x="16" y="80">
          C<tspan xmlSpace="default">{"   D   E "}</tspan>
          {" F "}
        </text>
      </g>
      <text x="16" y="120" xmlSpace="default">
        {"GH   I"}
      </text>
      <text x="16" y="160" xmlSpace="preserve">
        {"J K   L"}
      </text>
    </svg>
  );
}
