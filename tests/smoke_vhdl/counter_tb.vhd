library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity counter_tb is
end entity;

architecture sim of counter_tb is
    constant WIDTH : integer := 8;
    signal clk   : std_logic := '0';
    signal rst_n : std_logic := '0';
    signal q     : unsigned(WIDTH-1 downto 0);
begin
    clk <= not clk after 5 ns;

    dut : entity work.counter
        generic map (WIDTH => WIDTH)
        port map (clk => clk, rst_n => rst_n, q => q);

    stim : process
    begin
        wait for 12 ns;
        rst_n <= '1';
        wait for 40 ns;
        if q /= to_unsigned(4, WIDTH) then
            report "FAIL: q = " & integer'image(to_integer(q)) & ", expected 4"
                severity failure;
        end if;
        report "PASS: counter_tb";
        std.env.finish;
    end process;
end architecture;
