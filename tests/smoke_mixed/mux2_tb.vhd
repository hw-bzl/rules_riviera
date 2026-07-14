library ieee;
use ieee.std_logic_1164.all;

entity mux2_tb is
end entity;

architecture sim of mux2_tb is
    component mux2 is
        port (sel, a, b : in std_logic; y : out std_logic);
    end component;

    signal sel, a, b, y : std_logic := '0';
begin
    dut : mux2 port map (sel => sel, a => a, b => b, y => y);

    stim : process
    begin
        a <= '0'; b <= '1'; sel <= '0';
        wait for 1 ns;
        assert y = '0' report "FAIL: sel=0 should pick a=0" severity failure;

        sel <= '1';
        wait for 1 ns;
        assert y = '1' report "FAIL: sel=1 should pick b=1" severity failure;

        report "PASS: mux2_tb";
        std.env.finish;
    end process;
end architecture;
