module adder_tb;
    localparam int WIDTH = 8;

    logic [WIDTH-1:0] a;
    logic [WIDTH-1:0] b;
    logic [WIDTH:0]   sum;

    adder #(.WIDTH(WIDTH)) dut (.a(a), .b(b), .sum(sum));

    initial begin
        a = 8'd3;
        b = 8'd4;
        #1;
        if (sum !== 9'd7) begin
            $display("FAIL: 3 + 4 = %0d, expected 7", sum);
            $fatal(1, "adder mismatch");
        end

        a = 8'hff;
        b = 8'h01;
        #1;
        if (sum !== 9'h100) begin
            $display("FAIL: 0xff + 0x01 = %h, expected 0x100", sum);
            $fatal(1, "adder overflow mismatch");
        end

        $display("PASS: adder_tb");
        $finish;
    end
endmodule
