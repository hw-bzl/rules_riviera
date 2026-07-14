module mux2 (
    input  logic sel,
    input  logic a,
    input  logic b,
    output logic y
);
    assign y = sel ? b : a;
endmodule
