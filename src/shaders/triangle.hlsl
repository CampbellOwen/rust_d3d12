cbuffer Camera : register(b0) {
    float4x4 m;
}

struct PSInput
{
    float4 position : SV_POSITION;
    float4 color : COLOR;
};

PSInput VSMain(float3 position : POSITION, float3 normal : NORMAL, float2 uv : TEXCOORD)
{
    PSInput result;

    result.position = mul(m, float4(position, 1.0));
    //result.color = mul(m, color);
    result.color = float4(0.2, 0.2, 0.2, 1.0);

    return result;
}

float4 PSMain(PSInput input) : SV_TARGET
{
    return input.color;
}