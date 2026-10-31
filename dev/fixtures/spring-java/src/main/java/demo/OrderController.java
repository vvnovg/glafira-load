package demo;

import org.springframework.web.bind.annotation.*;

@RestController
@RequestMapping("/v1/orders")
public class OrderController {

    @PostMapping
    public OrderDto create(@RequestBody CreateOrderRequest request) {
        return null;
    }

    @GetMapping("/{id}")
    public OrderDto get(@PathVariable("id") String id, @RequestParam(required = false) String expand) {
        return null;
    }

    @RequestMapping(method = RequestMethod.DELETE, path = "/{id}")
    public void delete(@PathVariable String id) {
    }

    @GetMapping(Paths.SEARCH)
    public java.util.List<OrderDto> search() {
        return null;
    }
}
