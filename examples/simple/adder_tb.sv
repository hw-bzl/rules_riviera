module adder_tb;
    logic [7:0] a, b;
    logic [8:0] sum;

    adder #(.WIDTH(8)) dut (.a(a), .b(b), .sum(sum));

    initial begin
        a = 8'd3; b = 8'd4; #1;
        if (sum !== 9'd7) $fatal(1, "adder mismatch (3+4)");
        a = 8'hff; b = 8'h01; #1;
        if (sum !== 9'h100) $fatal(1, "adder mismatch (0xff+1)");
        $display("PASS: adder_tb");
        $finish;
    end
endmodule
